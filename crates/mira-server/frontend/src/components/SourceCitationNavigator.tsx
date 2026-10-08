import { useEffect, useRef, useState, type RefObject } from 'react';
import { parseCitation, resolveCitationRange, type SourceCitation } from '../lib/sourceCitations';

export function SourceCitationNavigator({ pane, sessionId, hasOlder, loading, loadOlder, historyError, onOpenSession }: {
  pane: RefObject<HTMLDivElement | null>; sessionId: string | null; hasOlder: boolean; loading: boolean; loadOlder: () => void; historyError: string | null; onOpenSession: (id: string) => void;
}) {
  const [target, setTarget] = useState<SourceCitation | null>(null);
  const [error, setError] = useState<string | null>(null);
  const active = useRef<SourceCitation | null>(null);
  const fade = useRef<number | undefined>(undefined);
  useEffect(() => {
    const click = (event: MouseEvent) => {
      const link = (event.target as Element)?.closest?.('a');
      const citation = parseCitation(link?.getAttribute('href') ?? '');
      if (!citation) return;
      event.preventDefault(); event.stopPropagation(); active.current = citation; setTarget(citation); setError(null);
      if (citation.session !== sessionId) onOpenSession(citation.session);
    };
    document.addEventListener('click', click, true);
    return () => document.removeEventListener('click', click, true);
  }, [sessionId]);
  useEffect(() => {
    if (!target || target.session !== sessionId || !pane.current) return;
    if (historyError) { setError(historyError); setTarget(null); return; }
    let frame = 0; let attempts = 0; let stopped = false;
    const locate = () => {
      if (stopped || !pane.current) return;
      const turn = Array.from(pane.current.querySelectorAll<HTMLElement>('[data-minimap-id]')).find(node => node.dataset.minimapId === target.turn);
      if (turn) {
        for (const source of turn.querySelectorAll<HTMLElement>('[data-assistant-message]')) {
          const range = resolveCitationRange(source, target);
          if (!range) continue;
          range.startContainer.parentElement?.scrollIntoView({ block: 'center', behavior: 'smooth' });
          const highlights = (CSS as unknown as { highlights?: Map<string, unknown> }).highlights;
          const HighlightClass = (window as unknown as { Highlight?: new (range: Range) => unknown }).Highlight;
          if (highlights && HighlightClass) highlights.set('mira-citation', new HighlightClass(range));
          else { window.getSelection()?.removeAllRanges(); window.getSelection()?.addRange(range); }
          // A pulse on the reply draws the eye to where the passage landed;
          // the highlight itself fades after a few seconds.
          source.classList.remove('citation-flash');
          void source.offsetWidth;
          source.classList.add('citation-flash');
          window.clearTimeout(fade.current);
          fade.current = window.setTimeout(() => {
            highlights?.delete('mira-citation');
            source.classList.remove('citation-flash');
          }, 3200);
          setTarget(null); return;
        }
        setError('The quoted passage has changed or is no longer available.'); setTarget(null); return;
      }
      window.dispatchEvent(new CustomEvent('mira:transcript-jump', { detail: target.turn }));
      if (++attempts < 12) frame = requestAnimationFrame(locate);
      else if (hasOlder) { if (!loading) loadOlder(); }
      else { setError('The original reply is no longer available.'); setTarget(null); }
    };
    locate();
    return () => { stopped = true; cancelAnimationFrame(frame); };
  }, [target, sessionId, hasOlder, loading, loadOlder, historyError]);
  useEffect(() => {
    const clear = () => (CSS as unknown as { highlights?: Map<string, unknown> }).highlights?.delete('mira-citation');
    // A click on another citation link starts a new jump; anything else
    // dismisses the highlight early.
    const down = (event: PointerEvent) => {
      if (parseCitation((event.target as Element)?.closest?.('a')?.getAttribute('href') ?? '')) return;
      clear();
    };
    document.addEventListener('pointerdown', down); document.addEventListener('keydown', clear);
    return () => { clear(); window.clearTimeout(fade.current); document.removeEventListener('pointerdown', down); document.removeEventListener('keydown', clear); };
  }, [sessionId]);
  return error ? <div role="alert" className="flex items-center gap-2 text-xs text-muted-foreground">{error}<button onClick={() => setError(null)}>Dismiss</button></div> : target ? <div role="status" className="text-xs text-muted-foreground">Finding original reply…</div> : null;
}
