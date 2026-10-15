import type { Entry } from '../transcript/entries';
import type { TurnDiffSummary } from '../types';
export type ReviewTurnSource = { summary: TurnDiffSummary; key: string; label: string; timestamp?: number };
/** Reverse occurrences match the API; absolute transcript ordinals remain stable as new turns arrive. */
export function reviewTurnSources(entries: Entry[], summaries: TurnDiffSummary[]): ReviewTurnSource[] {
  const users = entries.filter(entry => entry.kind === 'msg' && entry.msg.role === 'user');
  const occurrences = new Map<string, number>();
  const result: ReviewTurnSource[] = [];
  for (let index = users.length - 1; index >= 0; index--) {
    const entry = users[index];
    if (entry.kind !== 'msg') continue;
    const text = entry.msg.content ?? '';
    const occurrence = occurrences.get(text) ?? 0;
    occurrences.set(text, occurrence + 1);
    const summary = summaries.find(turn => turn.text === text && turn.occurrence === occurrence);
    if (!summary?.files.length) continue;
    const ordinal = entry.transcriptTurnIndex ?? index;
    result.push({ summary, key: `turn:${ordinal}`, label: `Turn ${ordinal + 1}`, timestamp: entry.msg.created_at ?? undefined });
  }
  for (const summary of summaries) {
    if (summary.files.length && !result.some(item => item.summary === summary)) result.push({ summary, key: JSON.stringify(['unresolved',summary.text,summary.occurrence,users.length]), label: 'Earlier turn' });
  }
  return result;
}
