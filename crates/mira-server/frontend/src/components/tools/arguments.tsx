import type { DiffPreview, ToolCall } from '../../types';
import { infoFor } from '../ToolGroup';
import { ToolStatus } from './types';
export function readableArgRows(tool: string, args: any): { label: string; value: string }[] {
  if (!args || typeof args !== 'object') return [];
  const rows: { label: string; value: string }[] = [];
  const add = (label: string, value: unknown) => {
    if (typeof value !== 'string' && typeof value !== 'number' && typeof value !== 'boolean')
      return;
    const text = String(value).trim();
    if (!text) return;
    rows.push({ label, value: text });
  };
  switch (tool) {
    case 'read_file':
    case 'write_file':
    case 'edit_file':
    case 'rustfmt':
      add('file', args.path ?? args.file_path);
      if (tool === 'write_file' && typeof args.content === 'string')
        add('content', summarizeText(args.content));
      return rows;
    case 'web_fetch':
      add('url', args.url);
      return rows;
    case 'web_search':
      add('query', args.query);
      return rows;
    case 'grep':
      add('pattern', args.pattern);
      add('path', args.path);
      add('glob', args.glob);
      return rows;
    case 'glob':
      add('pattern', args.pattern);
      add('path', args.path);
      return rows;
    default:
      add('target', args.path ?? args.file_path ?? args.url ?? args.query ?? args.pattern);
      return rows;
  }
}

export function summarizeText(text: string): string {
  const one = text.replace(/\s+/g, ' ').trim();
  return one.length > 120 ? `${one.slice(0, 120)}…` : one;
}

/** Verb + target + icon for a single tool row. Tense follows Codex:
 *  in-flight/pending calls read as present-continuous ("Reading foo.rs"),
 *  finished/denied calls read as past ("Read foo.rs"). Verb map is
 *  centralised in `ToolGroup.infoFor` so grouped and ungrouped rows stay
 *  in lockstep. Target extraction stays here because it depends on
 *  per-tool arg shape (`args.pattern` for grep, `args.command` for bash). */
export function summarize(
  call: ToolCall,
  status: ToolStatus,
): { verb: string; target: string; icon: React.ReactNode } {
  const info = infoFor(call.function.name);
  const active = status === 'pending' || status === 'running';
  const verb = active ? info.verbCont : info.verbPast;
  const Icon = info.Icon;
  const icon = <Icon aria-hidden="true" className="size-3.5 shrink-0" />;

  const args = safeParse(call.function.arguments);
  const target = pickTarget(call.function.name, args);
  return { verb, target, icon };
}

/** Per-tool target extractor. Uses the argument key most useful to a
 *  human skimming the row — the file path, the command, the query — so
 *  the row reads as `Verb <what>` at a glance. */
export function pickTarget(tool: string, args: any): string {
  switch (tool) {
    case 'read_file':
    case 'write_file':
    case 'edit_file':
    case 'rustfmt':
      return shortPath(args?.path);
    case 'bash':
      return shortCmd(args?.command);
    case 'grep':
      return quote(args?.pattern);
    case 'glob':
      return args?.pattern ? String(args.pattern) : '';
    case 'find_symbol':
    case 'find_references':
    case 'find_callers':
      return args?.name ? String(args.name) : args?.query ? String(args.query) : '';
    case 'task_create':
      return args?.subject ? String(args.subject) : '';
    case 'task_update':
      return args?.task_id ? `#${args.task_id}${args.status ? ` → ${args.status}` : ''}` : '';
    case 'task_get':
      return args?.task_id ? `#${args.task_id}` : '';
    case 'task_list':
      return '';
    case 'web_fetch':
      return args?.url ? String(args.url) : '';
    case 'web_search':
      return quote(args?.query);
    case 'git_diff':
    case 'git_log':
    case 'git_status':
      return '';
    case 'git_commit':
      return args?.message ? shortCmd(args.message) : '';
    case 'memory_read':
    case 'memory_search':
    case 'memory_append':
    case 'memory_edit':
    case 'memory_remember':
      return args?.path ? shortPath(args.path) : args?.query ? String(args.query) : '';
    case 'skill':
      // "Used <name> skill" reads naturally alongside "Ran <cmd>" and
      // "Read <path>" — the tool name would be redundant otherwise.
      return args?.name ? `${args.name} skill` : '';
    case 'agent':
      return args?.prompt ? shortCmd(String(args.prompt)) : '';
    case 'delegate':
      return args?.query ? shortCmd(String(args.query)) : '';
    case 'browser':
      return args?.url ? String(args.url) : args?.action ? String(args.action) : '';
    case 'tool_search':
      return quote(args?.query);
    case 'plan':
    case 'ask_user':
      return '';
    default:
      return '';
  }
}

export function safeParse(s: string): any {
  try {
    return JSON.parse(s);
  } catch {
    return null;
  }
}

/** Tools whose target reads as a file path / identifier — earns the
 *  inline-code pill so `Read foo.rs` matches the Codex look. Bash
 *  commands, grep patterns, task numbers, etc. stay as plain mono
 *  text (a pill around a shell command would look weird). */
export function isPathishTool(name: string): boolean {
  switch (name) {
    case 'read_file':
    case 'write_file':
    case 'edit_file':
    case 'rustfmt':
    case 'glob':
    case 'web_fetch':
    case 'memory_read':
    case 'memory_append':
    case 'memory_edit':
      return true;
    default:
      return false;
  }
}

export function shortPath(p: string | undefined): string {
  if (!p) return '';
  const parts = String(p).split('/');
  if (parts.length <= 3) return String(p);
  return '…/' + parts.slice(-2).join('/');
}

export function shortCmd(cmd: string | undefined): string {
  if (!cmd) return '';
  const one = String(cmd).replace(/\s+/g, ' ').trim();
  return one.length > 60 ? one.slice(0, 60) + '…' : one;
}

export function quote(s: string | undefined): string {
  return s ? `"${s}"` : '';
}

export function labelFor(k: DiffPreview['kind']): string {
  switch (k) {
    case 'edit':
      return 'edit';
    case 'overwrite':
      return 'overwrite';
    case 'create':
      return 'create';
  }
}

export function prettyPrint(json: string): string {
  try {
    return JSON.stringify(JSON.parse(json), null, 2);
  } catch {
    return json;
  }
}
