import { X } from 'lucide-react';
import type { BrowserDraftNote } from '../lib/richComposerDrafts';

function host(url: string) {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/**
 * A browser note waiting in the composer: a crop of the page around the
 * mark, where it is, and what the user wrote. Clicking it opens the whole
 * marked screenshot; × drops it. Sent with the message as text plus that
 * screenshot.
 */
export function BrowserNoteCard({ note, onOpen, onRemove }: {
  note: BrowserDraftNote;
  onOpen: () => void;
  onRemove: () => void;
}) {
  return (
    <div className="group relative flex min-w-0 max-w-full items-center gap-2.5 rounded-xl border border-border/70 bg-fg/[0.03] p-1.5 pr-8 transition-colors hover:border-border hover:bg-fg/[0.05]">
      <button
        type="button"
        title="Show the marked screenshot"
        onClick={onOpen}
        className="relative h-[42px] w-[63px] shrink-0 overflow-hidden rounded-lg border border-border/60 bg-white"
      >
        <img src={note.thumb} alt="" className="size-full object-cover" />
      </button>
      <button type="button" onClick={onOpen} className="min-w-0 flex-1 text-left">
        <span className="flex min-w-0 items-center gap-1.5 text-[10.5px] font-medium uppercase tracking-wide text-muted-foreground/70">
          <span aria-hidden className="size-2 shrink-0 rounded-[1px_50%_50%_50%] bg-mira-blue" />
          <span className="truncate">
            Browser note{note.url ? ` · ${host(note.url)}` : ''}
          </span>
        </span>
        <span className="line-clamp-1 text-[12.5px] leading-snug text-foreground/90">{note.comment}</span>
        {note.target && (
          <span className="block truncate text-[11.5px] leading-snug text-muted-foreground">{note.target}</span>
        )}
      </button>
      <button
        type="button"
        aria-label="Remove note"
        onClick={onRemove}
        className="absolute right-1.5 top-1.5 flex size-5 items-center justify-center rounded text-muted-foreground/70 transition-colors hover:bg-fg/[0.08] hover:text-foreground"
      >
        <X className="size-3.5" />
      </button>
    </div>
  );
}
