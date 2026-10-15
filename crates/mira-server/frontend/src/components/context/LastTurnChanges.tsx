import type { Entry } from '../../transcript/entries';
import type { TurnDiffSummary } from '../../types';
import { TurnChanges } from '../TurnChanges';

/** Find by transcript order, including repeated prompts, rather than repo order. */
export function latestEditedTurn(entries: Entry[], summaries: TurnDiffSummary[]) {
  const occurrences = new Map<string, number>();
  for (let index = entries.length - 1; index >= 0; index--) {
    const entry = entries[index];
    if (entry.kind !== 'msg' || entry.msg.role !== 'user') continue;
    const text = entry.msg.content ?? '';
    const occurrence = occurrences.get(text) ?? 0;
    occurrences.set(text, occurrence + 1);
    const summary = summaries.find(summary => summary.text === text && summary.occurrence === occurrence);
    if (summary?.files.length) return summary;
  }
  return null;
}

export function LastTurnChanges({ summary, onOpenFile, onReview }: {
  summary: TurnDiffSummary; onOpenFile: (path: string) => void; onReview: () => void;
}) {
  return <div className="py-2"><p className="mb-1 text-[11px] font-medium text-muted-foreground">Latest turn edits</p><TurnChanges summary={summary} onOpenFile={onOpenFile} onReview={onReview} /></div>;
}
