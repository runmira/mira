import { useMemo, useState } from 'react';
import {
  Brain,
  CaretDown,
  ChatCircleDots,
  CheckCircle,
  CircleNotch,
  ClipboardText,
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
  UsersThree,
  WarningCircle,
} from '@phosphor-icons/react';
import type { DiffPreview, ToolCall, ToolResult } from '../types';
import { ToolCard, type ToolStatus } from './ToolCard';
import { cn } from '@/lib/utils';

/** Flat modern chip for a run of consecutive tool calls — no card border,
 *  no background box. Single row:
 *
 *      Exploring 2 reads, 4 searches                                    ✓
 *
 *  Umbrella verb mirrors Codex's Exploring/Explored pattern:
 *    - homogeneous run (all same tool) → the tool's own verb (Read/Searched/…)
 *      trailed by the target list (`foo.rs, bar.rs, +1 more`)
 *    - heterogeneous read+search only → Exploring / Explored + count summary
 *    - anything else → Working / Worked + count summary
 *
 *  Counts read as `N reads, N searches, N writes, …` in a stable order so a
 *  glance tells the user WHAT was done, not just HOW MANY things were done.
 *
 *  Click expands to reveal the individual ToolCards, tightly stacked
 *  underneath with no extra chrome. Only rendered when the run has 2+
 *  entries AND none are pending — pending calls must render individually
 *  so the y/n prompt is unmissable. */
export function ToolGroup({
  entries,
  onOpenFile,
}: {
  entries: {
    call: ToolCall;
    preview: DiffPreview | null;
    status: ToolStatus;
    result: ToolResult | null;
    progressLines?: string[];
  }[];
  onOpenFile?: (path: string, diff: DiffPreview | null) => void;
}) {
  const [expanded, setExpanded] = useState(false);

  const running = entries.filter((e) => e.status === 'running').length;
  const errored = entries.filter((e) => e.result?.is_error === true).length;
  const done = entries.length - running - errored;
  const anyActive = running > 0;

  // Homogeneous run → use the tool's own verb + inline target list.
  // Heterogeneous run → Exploring/Working umbrella + categorized counts
  // so the reader sees "2 reads, 4 searches" instead of a mixed target
  // salad.
  const names = new Set(entries.map((e) => e.call.function.name));
  const homogeneous = names.size === 1;
  const counts = useMemo(() => countsByCategory(entries.map((e) => e.call)), [entries]);
  const exploratory = useMemo(
    () => entries.every((e) => isExploratoryCategory(categoryFor(e.call.function.name))),
    [entries],
  );
  const header = useMemo(() => {
    if (homogeneous) {
      const info = infoFor(entries[0].call.function.name);
      return { verb: anyActive ? info.verbCont : info.verbPast };
    }
    if (exploratory) {
      return { verb: anyActive ? 'Exploring' : 'Explored' };
    }
    return { verb: anyActive ? 'Working' : 'Worked' };
  }, [homogeneous, entries, anyActive, exploratory]);

  const trailingText = useMemo(() => {
    if (homogeneous) return previewLine(entries.map((e) => e.call), true);
    const summary = countsPhrase(counts);
    return { text: summary, title: summary };
  }, [entries, homogeneous, counts]);

  return (
    <div className="w-full max-w-[78%]">
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="group flex w-full min-w-0 items-center gap-1.5 rounded-md px-2 py-1 text-left text-[13px] transition-colors hover:bg-accent/40"
      >
        <span className="shrink-0 text-muted-foreground">{header.verb}</span>
        {homogeneous && <CountBadge n={entries.length} />}
        <span
          className={cn(
            'min-w-0 flex-1 truncate text-[12.5px]',
            // Homogeneous keeps the mono target-list look; heterogeneous
            // uses the tighter categorised phrase which reads better in
            // the UI's default sans-serif.
            homogeneous
              ? 'font-mono text-[12px] text-muted-foreground/85'
              : 'text-foreground/85',
          )}
          title={trailingText.title}
        >
          {trailingText.text}
        </span>
        <CaretDown
          weight="bold"
          className={cn(
            'size-3 shrink-0 text-foreground/70 transition-all',
            !expanded && '-rotate-90',
            !expanded && 'opacity-0 group-hover:opacity-100',
          )}
        />
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
              progressLines={e.progressLines}
              onDecide={() => {}}
              onOpenFile={onOpenFile}
            />
          ))}
        </div>
      )}
    </div>
  );
}

/** Small pill next to the verb: `3`. Fills the same role as a count in
 *  a chat sidebar — a glance says "these belong together". Only shown for
 *  homogeneous runs; heterogeneous runs already carry per-category counts
 *  inline in the header text. */
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

/* ---------- categorization ---------- */

/** High-level activity buckets used for the Codex-style
 *  "Exploring 2 reads, 4 searches" header text. Categories group tools
 *  by *what the user cares about seeing* — a `git_diff` reads the tree,
 *  a `memory_append` writes to disk, etc. — not by internal harness
 *  taxonomy. Extend when new tool families land. */
export type ToolCategory =
  | 'read'
  | 'search'
  | 'write'
  | 'edit'
  | 'run'
  | 'fetch'
  | 'skill'
  | 'other';

/** Order the header pieces are emitted in. Reads before searches before
 *  mutations matches the natural narrative ("I looked, then I acted");
 *  keeping the order stable also avoids the header dancing around as
 *  new calls stream in. */
const CATEGORY_ORDER: ToolCategory[] = [
  'read',
  'search',
  'write',
  'edit',
  'run',
  'fetch',
  'skill',
  'other',
];

/** Singular/plural label per bucket. Kept out of the switch below so
 *  pluralisation stays consistent everywhere the counts are surfaced
 *  (ToolGroup header, WorkedForChip summary). */
const CATEGORY_LABEL: Record<ToolCategory, { one: string; many: string }> = {
  read:   { one: 'read',    many: 'reads'    },
  search: { one: 'search',  many: 'searches' },
  write:  { one: 'write',   many: 'writes'   },
  edit:   { one: 'edit',    many: 'edits'    },
  run:    { one: 'command', many: 'commands' },
  fetch:  { one: 'fetch',   many: 'fetches'  },
  skill:  { one: 'skill',   many: 'skills'   },
  other:  { one: 'call',    many: 'calls'    },
};

/** Map a raw tool name to its user-facing bucket. Unknown tools fall
 *  through to `other` so a stray extension tool still shows up in the
 *  count rather than vanishing. */
export function categoryFor(name: string): ToolCategory {
  switch (name) {
    case 'read_file':
    case 'memory_read':
    case 'memory_search':
    case 'task_get':
    case 'task_list':
    case 'git_log':
    case 'git_status':
    case 'git_diff':
      return 'read';
    case 'grep':
    case 'glob':
    case 'find_symbol':
    case 'find_references':
    case 'find_callers':
    case 'web_search':
      return 'search';
    case 'write_file':
    case 'memory_append':
    case 'memory_edit':
    case 'memory_remember':
    case 'task_create':
    case 'task_update':
    case 'git_commit':
      return 'write';
    case 'edit_file':
      return 'edit';
    case 'bash':
    case 'rustfmt':
    case 'run_background':
    case 'kill_background':
      return 'run';
    case 'read_output':
      return 'read';
    case 'web_fetch':
      return 'fetch';
    case 'skill':
      return 'skill';
    default:
      return 'other';
  }
}

/** True for categories that read the world but don't mutate it —
 *  used to pick the "Exploring / Explored" umbrella verb over
 *  "Working / Worked" when a run only observed things. */
export function isExploratoryCategory(cat: ToolCategory): boolean {
  return cat === 'read' || cat === 'search' || cat === 'fetch';
}

/** Count how many calls land in each bucket. Returns a Map so callers
 *  can iterate in `CATEGORY_ORDER` deterministically. Empty buckets
 *  are omitted. */
export function countsByCategory(calls: ToolCall[]): Map<ToolCategory, number> {
  const out = new Map<ToolCategory, number>();
  for (const c of calls) {
    const cat = categoryFor(c.function.name);
    out.set(cat, (out.get(cat) ?? 0) + 1);
  }
  return out;
}

/** Render a `2 reads, 4 searches, 1 write` phrase from a category map.
 *  Empty input → empty string so callers can `if (phrase) …` cleanly. */
export function countsPhrase(counts: Map<ToolCategory, number>): string {
  const parts: string[] = [];
  for (const cat of CATEGORY_ORDER) {
    const n = counts.get(cat) ?? 0;
    if (n === 0) continue;
    const label = n === 1 ? CATEGORY_LABEL[cat].one : CATEGORY_LABEL[cat].many;
    parts.push(`${n} ${label}`);
  }
  return parts.join(', ');
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
    case 'find_references': return { verbPast: 'Found refs',  verbCont: 'Finding refs',  Icon: MagnifyingGlass };
    case 'find_callers':    return { verbPast: 'Found callers', verbCont: 'Finding callers', Icon: MagnifyingGlass };
    case 'task_create':     return { verbPast: 'Added task',  verbCont: 'Adding task',   Icon: NotePencil };
    case 'task_update':     return { verbPast: 'Updated task', verbCont: 'Updating task', Icon: NotePencil };
    case 'task_list':       return { verbPast: 'Listed tasks', verbCont: 'Listing tasks', Icon: FileText };
    case 'task_get':        return { verbPast: 'Read task',   verbCont: 'Reading task',  Icon: FileText };
    case 'bash':            return { verbPast: 'Ran',          verbCont: 'Running',          Icon: Terminal };
    case 'run_background':  return { verbPast: 'Spawned',      verbCont: 'Spawning',         Icon: Terminal };
    case 'read_output':     return { verbPast: 'Read output',  verbCont: 'Reading output',   Icon: Terminal };
    case 'kill_background': return { verbPast: 'Stopped',      verbCont: 'Stopping',         Icon: Terminal };
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
    case 'skill':           return { verbPast: 'Used',       verbCont: 'Using',         Icon: Sparkle };
    case 'ask_user':        return { verbPast: 'Asked you',  verbCont: 'Waiting on you', Icon: ChatCircleDots };
    case 'plan':            return { verbPast: 'Proposed a plan', verbCont: 'Drafting a plan', Icon: ClipboardText };
    case 'agent':           return { verbPast: 'Delegated',  verbCont: 'Delegating',    Icon: UsersThree };
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
