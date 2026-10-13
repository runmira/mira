import type { Entry } from '../transcript/entries';

/** Read-only outline used by non-transcript consumers. Streaming text is excluded. */
function boundary(entry: Entry): string {
  if (entry.kind === 'msg') return JSON.stringify([entry.kind, entry.msg.role,
    entry.msg.role === 'user' ? entry.msg.content : !!entry.msg.content?.trim(), entry.msg.tool_calls?.length]);
  if (entry.kind === 'tool') return JSON.stringify([entry.kind, entry.call.id,
    entry.call.function.name, entry.status, entry.plan, entry.askUser,
    entry.status === 'pending' ? [entry.call, entry.preview, entry.needs] : null]);
  return entry.kind;
}
export function transcriptOutline(entries: Entry[], previous?: Entry[]): Entry[] {
  if (previous && previous.length === entries.length && entries.every((entry, i) =>
    entry === previous[i] || boundary(entry) === boundary(previous[i]))) return previous;
  return entries;
}

export function subagentOutline(next: Map<string, import('../transcript/entries').SubagentStreamState>, previous?: typeof next): typeof next {
  if (previous && previous.size === next.size && [...next].every(([id, stream]) => {
    const before = previous.get(id);
    return before && before.done === stream.done && before.pendingReview === stream.pendingReview &&
      transcriptOutline(stream.entries, before.entries) === before.entries;
  })) return previous;
  return next;
}
