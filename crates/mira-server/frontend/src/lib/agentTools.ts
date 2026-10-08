/**
 * An external agent's tool call, as a Mira tool call.
 *
 * Agents name their tools differently (Claude Code's `Read` / `Bash` /
 * `Edit` with `file_path`; ACP adapters by kind with diff content), but
 * they do the same things Mira's own tools do. Translating them here means
 * an agent turn renders through exactly the same cards, groups, verbs and
 * activity summary ("Worked for 6s · 3 reads, 1 edit") as a turn Mira ran
 * itself — one transcript, whichever engine wrote it.
 */
import type { AcpToolCall, ToolCall, ToolResult } from '../types';
import { boundedOutput, MAX_TOOL_OUTPUT } from './nativeStream';

type Args = Record<string, unknown>;

/** Mira's `browser_*` tool names → the `browser` tool's actions. */
const BROWSER_VERB_ACTIONS: Record<string, string> = {
  open: 'navigate',
  press: 'key',
  resize: 'set_viewport',
  tabs: 'list_tabs',
};

const str = (v: unknown): string | undefined => (typeof v === 'string' && v ? v : undefined);

/** Mira's tool name and arguments for an agent's call. */
export function agentToolAsMira(call: AcpToolCall): { name: string; args: Args } {
  const input = (call.raw_input && typeof call.raw_input === 'object' ? call.raw_input : {}) as Args;
  const name = call.name ?? '';
  const path = str(input.file_path) ?? str(input.path) ?? str(input.notebook_path) ?? call.locations?.[0]?.path;
  const diff = call.content?.find((c) => c.type === 'diff') as
    | { type: 'diff'; path: string; old_text?: string | null; new_text?: string | null }
    | undefined;

  switch (name) {
    case 'Computer':
    case 'computer':
    case 'computer_use':
      return { name: 'computer', args: input };
    case 'Browser':
    case 'browser':
      return { name: 'browser', args: input };
    case 'Bash':
      return { name: 'bash', args: { command: input.command, description: input.description } };
    case 'BashOutput':
      return { name: 'read_output', args: { id: input.bash_id } };
    case 'KillShell':
    case 'KillBash':
      return { name: 'kill_background', args: { id: input.shell_id ?? input.bash_id } };
    case 'Read':
      return { name: 'read_file', args: { path, offset: input.offset, limit: input.limit } };
    case 'Write':
      return { name: 'write_file', args: { path, content: input.content } };
    case 'Edit':
      return { name: 'edit_file', args: { path, old_string: input.old_string, new_string: input.new_string } };
    case 'MultiEdit': {
      const edits = Array.isArray(input.edits) ? (input.edits as Args[]) : [];
      return {
        name: 'edit_file',
        args: {
          path,
          old_string: edits.map((e) => String(e.old_string ?? '')).join('\n⋯\n'),
          new_string: edits.map((e) => String(e.new_string ?? '')).join('\n⋯\n'),
        },
      };
    }
    case 'NotebookEdit':
      return { name: 'edit_file', args: { path, old_string: '', new_string: input.new_source } };
    case 'Glob':
      return { name: 'glob', args: { pattern: input.pattern, path: input.path } };
    case 'Grep':
      return { name: 'grep', args: { pattern: input.pattern, path: input.path, glob: input.glob } };
    case 'WebFetch':
      return { name: 'web_fetch', args: { url: input.url } };
    case 'WebSearch':
      return { name: 'web_search', args: { query: input.query } };
    case 'TodoWrite':
      return { name: 'todo_write', args: input };
    case 'Task':
    case 'Agent':
      return { name: 'delegate', args: { query: input.description ?? input.prompt, ...input } };
    case 'Skill':
      return { name: 'skill', args: { name: input.skill ?? input.name, ...input } };
    case 'ToolSearch':
      return { name: 'tool_search', args: { query: input.query, ...input } };
    case 'mcp__mira__browser':
      return { name: 'browser', args: input };
  }

  // Mira's single-purpose browser tools, however the harness prefixes them
  // (`browser_open`, `mcp__mira__browser_open`, `mira/browser_open`).
  const verb = /(?:^|__|[./])browser_([a-z_]+)$/.exec(name)?.[1];
  if (verb) {
    const action = BROWSER_VERB_ACTIONS[verb] ?? verb;
    return { name: 'browser', args: { action, ...input } };
  }

  // ACP adapters describe tools by kind rather than name.
  switch (call.kind) {
    case 'read':
      return { name: 'read_file', args: { path, ...input } };
    case 'edit':
      return {
        name: diff?.old_text ? 'edit_file' : 'write_file',
        args: diff
          ? { path: diff.path, old_string: diff.old_text ?? '', new_string: diff.new_text ?? '', content: diff.new_text ?? '' }
          : { path, ...input },
      };
    case 'execute':
      return { name: 'bash', args: { command: input.command ?? call.title, ...input } };
    case 'search':
      return { name: 'grep', args: { pattern: input.pattern ?? input.query ?? call.title, ...input } };
    case 'fetch':
      return { name: 'web_fetch', args: { url: input.url ?? call.title, ...input } };
  }
  return { name: name || call.title || 'tool', args: input };
}

export function agentCallToToolCall(call: AcpToolCall): ToolCall {
  const { name, args } = agentToolAsMira(call);
  // Drop undefined fields so the card's "args" view stays clean.
  const clean = Object.fromEntries(Object.entries(args).filter(([, v]) => v !== undefined));
  return { id: call.id, type: 'function', function: { name, arguments: JSON.stringify(clean) } };
}

/** The agent's status in Mira's terms. Never `pending`: in Mira that means
 *  "awaiting your approval", and an agent's queued call is not that. */
export function agentToolStatus(call: AcpToolCall): 'running' | 'complete' {
  return call.status === 'completed' || call.status === 'failed' ? 'complete' : 'running';
}

/** The call's output as a Mira tool result, once it has finished. */
export function agentToolResult(call: AcpToolCall): ToolResult | null {
  if (call.status !== 'completed' && call.status !== 'failed') return null;
  const text = (call.content ?? [])
    .map((c) => (c.type === 'content' ? c.text : c.type === 'diff' ? `Edited ${c.path}` : ''))
    .filter(Boolean)
    .join('\n');
  const raw = typeof call.raw_output === 'string' ? call.raw_output : '';
  return { call_id: call.id, content: text || raw, is_error: call.status === 'failed' };
}

/** Bound display-only stdout retained on a tool, preserving structured diffs. */
export function boundAgentOutput(call: AcpToolCall): AcpToolCall {
  let budget = MAX_TOOL_OUTPUT;
  const content = [...(call.content ?? [])].reverse().map((block) => {
    if (block.type !== 'content') return block;
    const bounded = budget === 0 ? '' : block.text.length > budget ? '[Earlier output truncated]\n' + block.text.slice(-budget) : block.text;
    budget = Math.max(0, budget - block.text.length);
    return { ...block, text: bounded };
  }).reverse();
  const raw = typeof call.raw_output === 'string' ? call.raw_output : call.raw_output == null ? null : JSON.stringify(call.raw_output);
  return { ...call, content, raw_output: raw && raw.length > MAX_TOOL_OUTPUT ? boundedOutput('', raw) : call.raw_output };
}
