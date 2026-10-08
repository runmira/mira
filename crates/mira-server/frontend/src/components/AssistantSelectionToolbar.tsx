import { citationHref, type SourceCitation } from '../lib/sourceCitations';
import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react';
import { createPortal } from 'react-dom';
import { Check, Copy, MessageCircleQuestion, Quote } from 'lucide-react';

type Captured = { text: string; turn: string; range: Range; citation: SourceCitation };
/** Viewport coordinates belong in a body portal: transcript transforms and
 * scroll containers must never change the selection toolbar's origin. */
export function AssistantSelectionToolbar({ pane, sessionId, onQuote, onAskAside }: {
  pane: RefObject<HTMLDivElement | null>; sessionId: string | null;
  onQuote: (text: string, turn: string, href: string) => void;
  /** Open the side chat about this passage. */
  onAskAside?: (text: string) => void;
}) {
  const [captured, setCaptured] = useState<Captured | null>(null);
  const [copied, setCopied] = useState(false);
  const toolbar = useRef<HTMLDivElement>(null);
  useEffect(() => {
    setCaptured(null);
    let dragging = false;
    let frame = 0;
    const capture = () => {
      frame = 0;
      const selection = window.getSelection();
      if (!selection || selection.isCollapsed || selection.rangeCount !== 1 || !pane.current) { setCaptured(null); return; }
      const range = selection.getRangeAt(0);
      const element = (node: Node) => node.nodeType === Node.ELEMENT_NODE ? node as Element : node.parentElement;
      const source = element(range.startContainer)?.closest('[data-assistant-message]');
      if (!source || source !== element(range.endContainer)?.closest('[data-assistant-message]') || !pane.current.contains(source)) { setCaptured(null); return; }
      const bounds = pane.current.getBoundingClientRect();
      const rect = range.getBoundingClientRect();
      const text = selection.toString().trim();
      if (!text || !rect.width || rect.bottom < bounds.top || rect.top > bounds.bottom) { setCaptured(null); return; }
      const turn = source.closest('[data-minimap-id]')?.getAttribute('data-minimap-id') ?? '';
      const prefix = document.createRange(); prefix.selectNodeContents(source); prefix.setEnd(range.startContainer, range.startOffset);
      const raw = range.toString(); const offset = prefix.toString().length; const full = source.textContent ?? '';
      setCaptured({ text, turn, range: range.cloneRange(), citation: { session: sessionId ?? '', turn, quote: raw, before: full.slice(Math.max(0, offset - 48), offset), after: full.slice(offset + raw.length, offset + raw.length + 48) } });
    };
    const update = () => { if (!dragging) { cancelAnimationFrame(frame); frame = requestAnimationFrame(capture); } };
    const down = (event: PointerEvent) => { if (toolbar.current?.contains(event.target as Node)) return; dragging = event.button === 0; setCaptured(null); };
    const up = () => { dragging = false; update(); };
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && toolbar.current) { window.getSelection()?.removeAllRanges(); setCaptured(null); }
      if (event.key === 'Tab' && !event.shiftKey && !event.altKey && !event.ctrlKey && !event.metaKey && toolbar.current && !toolbar.current.contains(event.target as Node)) {
        event.preventDefault(); toolbar.current.focus({ preventScroll: true });
      }
    };
    document.addEventListener('selectionchange', update);
    document.addEventListener('pointerdown', down);
    document.addEventListener('pointerup', up);
    document.addEventListener('keydown', escape);
    window.addEventListener('scroll', update, true);
    window.addEventListener('resize', update);
    return () => {
      cancelAnimationFrame(frame);
      document.removeEventListener('selectionchange', update);
      document.removeEventListener('pointerdown', down);
      document.removeEventListener('pointerup', up);
      document.removeEventListener('keydown', escape);
      window.removeEventListener('scroll', update, true);
      window.removeEventListener('resize', update);
    };
  }, [pane, sessionId]);
  useEffect(() => setCopied(false), [captured]);
  useLayoutEffect(() => {
    if (!captured || !toolbar.current) return;
    const rects = captured.range.getClientRects();
    const selection = rects.item(rects.length - 1) ?? captured.range.getBoundingClientRect();
    const action = toolbar.current.getBoundingClientRect();
    const x = Math.max(8, Math.min(selection.left + selection.width / 2 - action.width / 2, window.innerWidth - action.width - 8));
    const y = selection.top >= action.height + 8 ? selection.top - action.height - 8 : selection.bottom + 8;
    toolbar.current.style.left = `${x}px`;
    toolbar.current.style.top = `${Math.max(8, Math.min(y, window.innerHeight - action.height - 8))}px`;
  }, [captured]);
  if (!captured || !sessionId) return null;
  const done = () => { window.getSelection()?.removeAllRanges(); setCaptured(null); };
  const item = 'inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-[12px] font-medium text-foreground/85 transition-colors hover:bg-fg/[0.08] hover:text-foreground';
  return createPortal(
    <div
      ref={toolbar}
      role="toolbar"
      aria-label="Selected text"
      tabIndex={-1}
      // Keep the selection: pressing a button must not collapse it first.
      onPointerDown={(event) => event.preventDefault()}
      className="fixed z-[90] flex items-center gap-0.5 rounded-lg border border-border bg-popover p-0.5 shadow-lg shadow-shade/20"
    >
      <button type="button" className={item} onClick={() => { onQuote(captured.text, captured.turn, citationHref(captured.citation)); done(); }}>
        <Quote className="size-3.5 text-muted-foreground" />Quote
      </button>
      {onAskAside && (
        <button type="button" className={item} onClick={() => { onAskAside(captured.text); done(); }}>
          <MessageCircleQuestion className="size-3.5 text-muted-foreground" />Ask in side chat
        </button>
      )}
      <span aria-hidden className="mx-0.5 h-4 w-px bg-border" />
      <button
        type="button"
        className={item}
        aria-label="Copy"
        title="Copy"
        onClick={() => {
          void navigator.clipboard?.writeText(captured.text).then(() => {
            setCopied(true);
            window.setTimeout(done, 700);
          });
        }}
      >
        {copied ? <Check className="size-3.5 text-emerald-500" /> : <Copy className="size-3.5 text-muted-foreground" />}
        {copied ? 'Copied' : 'Copy'}
      </button>
    </div>,
    document.body,
  );
}
