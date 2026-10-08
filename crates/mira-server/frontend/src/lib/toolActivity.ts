import type { ToolCall } from '../types';
export function plainToolAction(call: ToolCall, active: boolean): string {
  let args: Record<string, unknown> = {};
  try { args = JSON.parse(call.function.arguments); } catch { /* incomplete streamed arguments */ }
  const value = (...keys: string[]) => keys.map(key => args[key]).find(value => typeof value === 'string') as string | undefined;
  const path = value('path', 'file_path', 'filePath', 'filename');
  const name = call.function.name.toLowerCase();
  const label = (present: string, past: string, target?: string) => `${active ? present : past}${target ? ` ${target}` : ''}`;
  if (/read.*file|^read$/.test(name)) return label('Reading', 'Read', path ?? 'a file');
  if (/write.*file|edit.*file|apply.*patch/.test(name)) return label('Updating', 'Updated', path ?? 'files');
  if (/grep|search|glob/.test(name)) return label('Searching', 'Searched', name.includes('web') ? 'the web' : 'files');
  if (/list.*file|^ls$/.test(name)) return label('Listing', 'Listed', 'files');
  if (/bash|shell|exec|command|run_background/.test(name)) {
    const command = value('command', 'cmd') ?? '';
    const description = value('description');
    if (description?.trim()) return description.trim().slice(0, 180);
    if (/\b(test|pytest|vitest|jest)\b/.test(command)) return label('Running', 'Ran', 'tests');
    if (/\b(build|tsc|check)\b/.test(command)) return label('Checking', 'Checked', 'the project');
    if (/git\s+(status|diff)/.test(command)) return label('Reviewing', 'Reviewed', 'repository changes');
    if (/\b(rg|grep|find)\b/.test(command)) return label('Searching', 'Searched', 'files');
    return label('Running', 'Ran', 'a command');
  }
  if (/fetch|browse|browser/.test(name)) return label('Opening', 'Opened', value('url') ?? 'a page');
  if (/git/.test(name)) return label('Checking', 'Checked', 'the repository');
  if (/agent|delegate/.test(name)) return label('Working with', 'Worked with', 'an agent');
  const humanName = name.replace(/[_/.-]+/g, ' ').trim();
  return label('Using', 'Used', humanName || 'a tool');
}
export function latestToolEntry<T extends { activityAt?: number; call: ToolCall }>(entries: T[]): T | undefined {
  return entries.reduce<T | undefined>((latest, entry) => !latest || (entry.activityAt ?? 0) >= (latest.activityAt ?? 0) ? entry : latest, undefined);
}

/** Shared across provider, native and MCP names; unknown tools retain an icon. */
export type ToolActivityKind = 'read' | 'search' | 'web_search' | 'edit' | 'write' | 'run' | 'web' | 'git' | 'agent' | 'memory' | 'task' | 'plan' | 'question' | 'compact' | 'skill' | 'other';
export function toolActivityKind(raw: string): ToolActivityKind {
  const tool = raw.startsWith('mcp__') ? raw.split('__').at(-1)! : raw;
  const name = tool.replace(/([a-z])([A-Z])/g, '$1_$2').toLowerCase();
  const word = name.replace(/[_/.: -]+/g, '');
  if (/compact|summarizecontext/.test(word)) return 'compact';
  if (/websearch|searchweb|internetsearch/.test(word)) return 'web_search';
  if (/askuser|requestuserinput|question/.test(word)) return 'question';
  if (/memory/.test(word)) return 'memory';
  if (/task|todo/.test(word) && !/delegate|agent/.test(word)) return 'task';
  if (/plan/.test(word)) return 'plan';
  if (/agent|delegate/.test(word)) return 'agent';
  if (/skill/.test(word)) return 'skill';
  if (/git|repository/.test(word)) return 'git';
  if (/bash|shell|exec|command|runbackground|terminal|rustfmt/.test(word)) return 'run';
  if (/read|listfiles|listdirectory|^ls$/.test(word)) return 'read';
  if (/grep|glob|search|findsymbol|findreferences|findcallers/.test(word)) return 'search';
  if (/edit|patch|replace/.test(word)) return 'edit';
  if (/write|createfile/.test(word)) return 'write';
  if (/fetch|browser|browse|url|navigate/.test(word)) return 'web';
  return 'other';
}
const completedPhrases: Record<ToolActivityKind, string> = {
  read: 'Read files', search: 'Searched files', web_search: 'Searched the web',
  edit: 'Edited files', write: 'Wrote files', run: 'Ran commands', web: 'Opened web pages',
  git: 'Reviewed the repository', agent: 'Worked with agents', memory: 'Used memory',
  task: 'Managed tasks', plan: 'Updated the plan', question: 'Asked questions',
  compact: 'Compacted context', skill: 'Used skills', other: 'Used tools',
};
export function completedToolSummary(entries: { call: ToolCall; status: string; result?: { is_error?: boolean } | null }[]): string {
  const succeeded = entries.filter(entry => entry.status === 'complete' && !entry.result?.is_error);
  if (!succeeded.length) return 'Tools did not complete';
  const kinds = [...new Set(succeeded.map(entry => toolActivityKind(entry.call.function.name)))];
  const parts = kinds.map(kind => kind === 'run' && succeeded.filter(entry => toolActivityKind(entry.call.function.name) === 'run').length === 1 ? 'Ran a command' : completedPhrases[kind]);
  const text = parts.map((part, index) => index ? part[0].toLowerCase() + part.slice(1) : part).join(', ');
  const failed = entries.some(entry => entry.result?.is_error || entry.status === 'denied' || entry.status === 'error');
  return text + (failed ? '; some tools did not complete' : '');
}
