import {
  Brain,
  ClipboardList,
  FilePlus,
  FileText,
  GitBranch,
  GitCommit,
  GitCompare,
  Globe,
  ListCollapse,
  MessageCircleMore,
  NotebookPen,
  Search,
  Sparkle,
  Terminal,
  Users,
  Wrench,
} from 'lucide-react';
import { toolActivityKind } from '../../lib/toolActivity';
import type { ToolCall } from '../../types';
/* ---------- categorization ---------- */

/** High-level activity buckets used for the Codex-style
 *  "Exploring 2 reads, 4 searches" header text. Categories group tools
 *  by *what the user cares about seeing* — a `git_diff` reads the tree,
 *  a `memory_append` writes to disk, etc. — not by internal harness
 *  taxonomy. Extend when new tool families land. */
export type ToolCategory =
  'read' | 'search' | 'write' | 'edit' | 'run' | 'fetch' | 'skill' | 'other';

/** Order the header pieces are emitted in. Reads before searches before
 *  mutations matches the natural narrative ("I looked, then I acted");
 *  keeping the order stable also avoids the header dancing around as
 *  new calls stream in. */
export const CATEGORY_ORDER: ToolCategory[] = [
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
export const CATEGORY_LABEL: Record<ToolCategory, { one: string; many: string }> = {
  read: { one: 'read', many: 'reads' },
  search: { one: 'search', many: 'searches' },
  write: { one: 'write', many: 'writes' },
  edit: { one: 'edit', many: 'edits' },
  run: { one: 'command', many: 'commands' },
  fetch: { one: 'fetch', many: 'fetches' },
  skill: { one: 'skill', many: 'skills' },
  other: { one: 'call', many: 'calls' },
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
    case 'browser':
      return 'fetch';
    case 'tool_search':
      return 'search';
    case 'todo_write':
      return 'write';
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

export type ToolInfo = {
  /** Past-tense verb, used once the call completes ("Read foo.rs"). */
  verbPast: string;
  /** Present-continuous verb, used while the call is in-flight
   *  ("Reading foo.rs"). Matches Codex's Searching/Searched pattern. */
  verbCont: string;
  Icon: React.ComponentType<{ className?: string; 'aria-hidden'?: boolean | 'true' | 'false' }>;
};

/** Friendly label + icon per tool name. Extend as we add tools; unknown
 *  names fall back to the raw name so nothing goes invisible. */
export function infoFor(name: string): ToolInfo {
  switch (name) {
    case 'read_file':
      return { verbPast: 'Read', verbCont: 'Reading', Icon: FileText };
    case 'write_file':
      return { verbPast: 'Wrote', verbCont: 'Writing', Icon: FilePlus };
    case 'edit_file':
      return { verbPast: 'Edited', verbCont: 'Editing', Icon: NotebookPen };
    case 'grep':
      return { verbPast: 'Searched', verbCont: 'Searching', Icon: Search };
    case 'glob':
      return { verbPast: 'Found', verbCont: 'Finding', Icon: Search };
    case 'find_symbol':
      return { verbPast: 'Found', verbCont: 'Finding', Icon: Search };
    case 'find_references':
      return { verbPast: 'Found refs', verbCont: 'Finding refs', Icon: Search };
    case 'find_callers':
      return { verbPast: 'Found callers', verbCont: 'Finding callers', Icon: Search };
    case 'task_create':
      return { verbPast: 'Added task', verbCont: 'Adding task', Icon: NotebookPen };
    case 'task_update':
      return { verbPast: 'Updated task', verbCont: 'Updating task', Icon: NotebookPen };
    case 'task_list':
      return { verbPast: 'Listed tasks', verbCont: 'Listing tasks', Icon: FileText };
    case 'task_get':
      return { verbPast: 'Read task', verbCont: 'Reading task', Icon: FileText };
    case 'bash':
      return { verbPast: 'Ran', verbCont: 'Running', Icon: Terminal };
    case 'run_background':
      return { verbPast: 'Spawned', verbCont: 'Spawning', Icon: Terminal };
    case 'read_output':
      return { verbPast: 'Read output', verbCont: 'Reading output', Icon: Terminal };
    case 'kill_background':
      return { verbPast: 'Stopped', verbCont: 'Stopping', Icon: Terminal };
    case 'rustfmt':
      return { verbPast: 'Formatted', verbCont: 'Formatting', Icon: Sparkle };
    case 'web_fetch':
      return { verbPast: 'Fetched', verbCont: 'Fetching', Icon: Globe };
    case 'web_search':
      return { verbPast: 'Searched the web', verbCont: 'Searching the web', Icon: Globe };
    case 'git_diff':
      return { verbPast: 'Diffed', verbCont: 'Diffing', Icon: GitCompare };
    case 'git_status':
      return { verbPast: 'Checked status', verbCont: 'Checking status', Icon: GitBranch };
    case 'git_log':
      return { verbPast: 'Read log', verbCont: 'Reading log', Icon: GitCommit };
    case 'git_commit':
      return { verbPast: 'Committed', verbCont: 'Committing', Icon: GitCommit };
    case 'memory_read':
      return { verbPast: 'Recalled', verbCont: 'Recalling', Icon: Brain };
    case 'memory_search':
      return { verbPast: 'Recalled', verbCont: 'Recalling', Icon: Brain };
    case 'memory_append':
      return { verbPast: 'Remembered', verbCont: 'Remembering', Icon: Brain };
    case 'memory_edit':
      return { verbPast: 'Remembered', verbCont: 'Remembering', Icon: Brain };
    case 'memory_remember':
      return { verbPast: 'Remembered', verbCont: 'Remembering', Icon: Brain };
    case 'skill':
      return { verbPast: 'Used', verbCont: 'Using', Icon: Sparkle };
    case 'ask_user':
      return { verbPast: 'Asked you', verbCont: 'Waiting on you', Icon: MessageCircleMore };
    case 'plan':
      return { verbPast: 'Proposed a plan', verbCont: 'Drafting a plan', Icon: ClipboardList };
    case 'agent':
      return { verbPast: 'Delegated', verbCont: 'Delegating', Icon: Users };
    case 'delegate_task':
      return { verbPast: 'Handed off', verbCont: 'Handing off', Icon: Users };
    // Tools an external agent has that Mira's harness doesn't name.
    case 'delegate':
      return { verbPast: 'Delegated', verbCont: 'Delegating', Icon: Users };
    case 'browser':
      return { verbPast: 'Browsed', verbCont: 'Browsing', Icon: Globe };
    case 'todo_write':
      return { verbPast: 'Updated todos', verbCont: 'Updating todos', Icon: ClipboardList };
    case 'tool_search':
      return { verbPast: 'Looked up tools', verbCont: 'Looking up tools', Icon: Search };
    default: {
      const kind = toolActivityKind(name);
      const fallback: Record<ReturnType<typeof toolActivityKind>, ToolInfo> = {
        read: { verbPast: 'Read', verbCont: 'Reading', Icon: FileText },
        search: { verbPast: 'Searched', verbCont: 'Searching', Icon: Search },
        web_search: { verbPast: 'Searched the web', verbCont: 'Searching the web', Icon: Search },
        edit: { verbPast: 'Edited', verbCont: 'Editing', Icon: NotebookPen },
        write: { verbPast: 'Wrote', verbCont: 'Writing', Icon: FilePlus },
        run: { verbPast: 'Ran', verbCont: 'Running', Icon: Terminal },
        web: { verbPast: 'Opened', verbCont: 'Opening', Icon: Globe },
        git: { verbPast: 'Reviewed', verbCont: 'Reviewing', Icon: GitBranch },
        agent: { verbPast: 'Worked with agents', verbCont: 'Working with agents', Icon: Users },
        memory: { verbPast: 'Used memory', verbCont: 'Using memory', Icon: Brain },
        task: { verbPast: 'Managed tasks', verbCont: 'Managing tasks', Icon: ClipboardList },
        plan: { verbPast: 'Updated the plan', verbCont: 'Planning', Icon: ClipboardList },
        question: { verbPast: 'Asked', verbCont: 'Asking', Icon: MessageCircleMore },
        compact: {
          verbPast: 'Compacted context',
          verbCont: 'Compacting context',
          Icon: ListCollapse,
        },
        skill: { verbPast: 'Used skills', verbCont: 'Using skills', Icon: Sparkle },
        other: {
          verbPast: `Used ${name.replace(/[_/.-]+/g, ' ')}`,
          verbCont: `Using ${name.replace(/[_/.-]+/g, ' ')}`,
          Icon: Wrench,
        },
      };
      return fallback[kind];
    }
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
  } catch {
    /* ignore */
  }
  return '';
}
