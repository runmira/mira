import { Quote, X } from 'lucide-react';
import type { ComposerQuote } from '@/lib/attachBridge';

/** Which reply a quote came from, as the card labels it. */
export function quoteSource(turn: string): string {
  const n = Number(turn.replace('turn-', ''));
  return turn && Number.isFinite(n) ? `Reply ${n + 1}` : 'Reply';
}

/** The text a quote becomes when the message is sent: a linked source line
 *  and the passage as a blockquote. The link is what lets a reader of the
 *  sent message jump back to the original. */
export function quoteMarkdown(q: ComposerQuote): string {
  const lines = q.text.trim().split('\n').map((line) => `> ${line}`).join('\n');
  return `Regarding [${quoteSource(q.turn).toLowerCase()}](${q.href}):\n${lines}\n\n`;
}

/**
 * A quoted passage waiting in the composer. Clicking anywhere on the card
 * jumps to (and highlights) the original in the chat; × drops it.
 *
 * The jump target is a stretched citation link covering the whole card, so
 * the icon, text, and padding all navigate. The remove button sits *above*
 * (and outside) that link: `SourceCitationNavigator` hijacks clicks inside
 * a citation anchor at document-capture time, so a button nested in the
 * link would jump instead of removing. Siblings keep both actions working
 * with valid HTML (no nested interactives).
 */
export function QuoteCard({ quote, onRemove }: { quote: ComposerQuote; onRemove: () => void }) {
  return (
    <div className="group relative flex min-w-0 max-w-full items-start gap-2 rounded-lg border border-border/70 bg-fg/[0.03] py-1.5 pl-2.5 pr-7 transition-colors hover:border-border hover:bg-fg/[0.05]">
      <span aria-hidden className="absolute inset-y-1.5 left-0 w-[3px] rounded-full bg-mira-blue/60" />
      <Quote className="mt-0.5 size-3 shrink-0 text-mira-blue/80" aria-hidden />
      <span className="min-w-0 flex-1">
        <span className="block text-[10.5px] font-medium uppercase tracking-wide text-muted-foreground/70">
          {quoteSource(quote.turn)}
        </span>
        <span className="line-clamp-2 text-[12.5px] leading-snug text-foreground/85">{quote.text}</span>
      </span>
      <a
        href={quote.href}
        title="Show in chat"
        aria-label="Show in chat"
        className="absolute inset-0 rounded-lg no-underline"
      />
      <button
        type="button"
        aria-label="Remove quote"
        onClick={(e) => {
          e.preventDefault();
          e.stopPropagation();
          onRemove();
        }}
        className="absolute right-1.5 top-1.5 z-10 flex size-5 items-center justify-center rounded text-muted-foreground/70 transition-colors hover:bg-fg/[0.08] hover:text-foreground"
      >
        <X className="size-3.5" />
      </button>
    </div>
  );
}
