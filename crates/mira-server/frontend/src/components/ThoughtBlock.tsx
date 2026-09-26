import { useEffect, useRef, useState } from 'react';
import { Brain, CaretDown } from '@phosphor-icons/react';
import { Markdown } from './Markdown';
import { cn } from '@/lib/utils';

/**
 * The model's reasoning as a collapsible transcript section.
 *
 *   live     → "Thinking… 4s"  open, streaming text in a short scroller
 *   settled  → "Thought for 4s ⌄"  collapsed; click to read the whole thing
 *
 * Fed either by structured `reasoning` frames (Anthropic extended
 * thinking, Gemini thought summaries, DeepSeek / OpenRouter reasoning)
 * or by inline `<think>` tags some open models emit in their text.
 */
export function ThoughtBlock({
  content,
  live,
  startedAt,
  endedAt,
}: {
  content: string;
  live: boolean;
  /** Epoch ms. Omitted for reasoning restored from history — the
   *  duration wasn't recorded, so the header just reads "Thought". */
  startedAt?: number | null;
  endedAt?: number | null;
}) {
  const [open, setOpen] = useState(live);
  // Fold away once thinking ends, unless the reader opened / closed it
  // themselves in the meantime.
  const touched = useRef(false);
  useEffect(() => {
    if (!live && !touched.current) setOpen(false);
  }, [live]);

  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!live) return;
    const id = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(id);
  }, [live]);

  // While live the cell is a fixed-height window that follows the newest
  // line as text streams in — like the chat pane — unless the reader has
  // scrolled up inside it to re-read, in which case it stays put until
  // they scroll back to the bottom.
  const bodyRef = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  useEffect(() => {
    const el = bodyRef.current;
    if (live && el && following.current) el.scrollTop = el.scrollHeight;
  }, [content, live, open]);
  const onScroll = () => {
    const el = bodyRef.current;
    if (el) following.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
  };

  const secs =
    startedAt != null ? Math.max(0, Math.round(((endedAt ?? now) - startedAt) / 1000)) : null;
  const dur = secs == null ? null : secs < 1 ? '<1s' : formatElapsed(secs);
  const label = live
    ? 'Thinking…'
    : dur
      ? `Thought for ${dur}`
      : 'Thought';
  const body = content.trim();

  return (
    <div className="my-1">
      <button
        type="button"
        onClick={() => {
          touched.current = true;
          setOpen((v) => !v);
        }}
        disabled={!body}
        aria-expanded={open}
        className="group inline-flex items-center gap-1.5 rounded px-1 py-0.5 text-[12.5px] text-muted-foreground transition-colors hover:text-foreground disabled:cursor-default"
      >
        <Brain className={cn('size-3.5 shrink-0', live && 'animate-pulse text-mira-purple')} />
        <span className={cn('font-medium', live && 'text-foreground/80')}>{label}</span>
        {live && dur && <span className="font-mono text-[11px] text-muted-foreground/70">{dur}</span>}
        {body && (
          <CaretDown
            weight="bold"
            className={cn(
              'size-3 text-muted-foreground/60 transition-transform',
              !open && '-rotate-90',
            )}
          />
        )}
      </button>
      {open && body && (
        <div
          ref={bodyRef}
          onScroll={live ? onScroll : undefined}
          className={cn(
            'thought ml-[7px] mt-1 animate-fade-in border-l-2 border-border pl-3 text-[13px] leading-relaxed text-muted-foreground',
            // Fade the top edge so older lines read as scrolling away.
            live &&
              'max-h-40 overflow-y-auto [mask-image:linear-gradient(to_bottom,transparent,black_2rem)]',
          )}
        >
          <Markdown text={body} />
        </div>
      )}
    </div>
  );
}

function formatElapsed(secs: number): string {
  if (secs < 60) return `${secs}s`;
  return `${Math.floor(secs / 60)}m ${secs % 60}s`;
}
