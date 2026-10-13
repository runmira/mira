import type { Entry } from '../transcript/entries';
/** Finish only the user turn owning this terminal, preserving a queued follow-up. */
export function terminalEntries(entries: Entry[], inputId: string | undefined, apply: (entries: Entry[]) => Entry[]) {
  if (!inputId || inputId.startsWith('local:')) return apply(entries);
  const start = entries.findIndex(entry => entry.kind === 'msg' && entry.msg.role === 'user' && entry.msg.input_id === inputId);
  if (start < 0) return entries;
  const next = entries.findIndex((entry, index) => index > start && entry.kind === 'msg' && entry.msg.role === 'user' && entry.msg.input_intent !== 'steer');
  const end = next < 0 ? entries.length : next;
  return [...entries.slice(0, start), ...apply(entries.slice(start, end)), ...entries.slice(end)];
}
