import { useState } from 'react';
import type { Entry } from '../../transcript/entries';
import type { TurnDiffSummary } from '../../types';
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '../ui/dialog';
import { TranscriptSessionContext } from '../TranscriptDisclosure';
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

export function LastTurnChanges({ summary, session, onOpenFile }: {
  summary: TurnDiffSummary; session: string; onOpenFile: (path: string) => void;
}) {
  const [open, setOpen] = useState(false);
  return <div className="py-2">
    <button type="button" onClick={() => setOpen(true)} className="flex w-full items-center justify-between rounded-md py-1.5 text-left text-[12px] text-fg/65 hover:text-fg/90">
      <span>Latest turn edits</span><span>{summary.files.length} file{summary.files.length === 1 ? '' : 's'}</span>
    </button>
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="max-h-[85vh] max-w-3xl overflow-y-auto">
        <DialogHeader><DialogTitle>Latest turn edits</DialogTitle></DialogHeader>
        <TranscriptSessionContext.Provider value={session}>
          <TurnChanges summary={summary} onOpenFile={onOpenFile} />
        </TranscriptSessionContext.Provider>
      </DialogContent>
    </Dialog>
  </div>;
}
