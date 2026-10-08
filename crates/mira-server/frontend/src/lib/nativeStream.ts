import type { Entry } from '../App';

/** Native IDs are scoped to the current user turn. Preserve message order
 * when deltas interleave with work; legacy frames use appendToken. */
export function appendNativeText(entries: Entry[], text: string, messageId: string, snapshot = false): Entry[] {
  if (!text) return entries;
  for (let i = entries.length - 1; i >= 0; i--) {
    const entry = entries[i];
    if (entry.kind === 'msg' && entry.msg.role === 'user') break;
    if (entry.kind === 'msg' && entry.nativeMessageId === messageId) {
      if ((!snapshot && entry.nativeCompleted) || (snapshot && entry.nativeCompleted && entry.msg.content === text)) return entries;
      return [...entries.slice(0, i), { ...entry, nativeCompleted: snapshot || entry.nativeCompleted, msg: { ...entry.msg, content: snapshot ? text : (entry.msg.content ?? '') + text } }, ...entries.slice(i + 1)];
    }
  }
  return [...entries, { kind: 'msg', nativeMessageId: messageId, nativeCompleted: snapshot, msg: { role: 'assistant', content: text, created_at: Date.now() } }];
}

export const MAX_TOOL_OUTPUT = 64 * 1024;
/** Preserve a bounded tail, including when one provider frame is oversized. */
export function boundedOutput(previous: string, delta: string): string {
  const text = previous + delta;
  return text.length <= MAX_TOOL_OUTPUT ? text : '[Earlier output truncated]\n' + text.slice(-MAX_TOOL_OUTPUT);
}

export function appendNativeToolOutput(entries: Entry[], id: string, delta: string): Entry[] {
  if (!delta) return entries;
  for (let i = entries.length - 1; i >= 0; i--) {
    const tool = entries[i];
    if (tool.kind !== 'tool' || tool.call.id !== id) continue;
    if (tool.status === 'complete' || tool.status === 'denied') return entries;
    const output = boundedOutput(tool.result?.content ?? '', delta);
    return [...entries.slice(0, i), { ...tool, result: { call_id: id, content: output, is_error: false } }, ...entries.slice(i + 1)];
  }
  return entries;
}

/** Provider item IDs belong to one user turn, including their metadata. */
export function applyNativeMetadata(entries: Entry[], messageId: string, phase: string): Entry[] {
  for (let index = entries.length - 1; index >= 0; index--) {
    const entry = entries[index];
    if (entry.kind === 'msg' && entry.msg.role === 'user') break;
    if (entry.kind === 'msg' && entry.nativeMessageId === messageId) {
      if (entry.nativePhase === phase) return entries;
      return [...entries.slice(0, index), { ...entry, nativePhase: phase }, ...entries.slice(index + 1)];
    }
  }
  return entries;
}
