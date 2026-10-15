import { Markdown } from './Markdown';
import { STREAM_ROW_GUTTER } from '../lib/streamLayout';
import { ThinkingPreview } from './ThinkingPreview';
import { ActivityShimmer } from './ActivityShimmer';
import { thinkingScrollEdges } from '../lib/thinkingPreview';
import { useTranscriptDisclosure } from './TranscriptDisclosure';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Brain, ChevronDown } from 'lucide-react';
import { Collapse } from './ui/Collapse';
import { cn } from '@/lib/utils';

/**
 * The model's reasoning as a collapsible transcript section.
 *
 *   live     → "Thinking… 4s" with a compact preview; expand to read
 *   settled  → "Thought for 4s ⌄"  collapsed; click to read the whole thing
 *
 * Fed either by structured `reasoning` frames (Anthropic extended
 * thinking, Gemini thought summaries, DeepSeek / OpenRouter reasoning)
 * or by inline `<think>` tags some open models emit in their text.
 */
export function ThoughtBlock({
  content,
  onOpenFile,
  live,
  startedAt,
  endedAt,
}: {
  content: string;
  onOpenFile?: (path: string, diff: import('../types').DiffPreview | null) => void;
  live: boolean;
  /** Epoch ms. Omitted for reasoning restored from history — the
   *  duration wasn't recorded, so the header just reads "Thought". */
  startedAt?: number | null;
  endedAt?: number | null;
}) {
  const [open, setOpen] = useTranscriptDisclosure(`thought:${startedAt ?? content.slice(0, 64)}`);
  // Fold away once thinking ends, unless the reader opened / closed it
  // themselves in the meantime.
  const touched = useRef(open);
  useEffect(() => {
    if (!live && !touched.current) setOpen(false);
  }, [live]);

  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!live) return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [live]);

  // While live the cell is a fixed-height window that follows the newest
  // line as text streams in — like the chat pane — unless the reader has
  // scrolled up inside it to re-read, in which case it stays put until
  // they scroll back to the bottom.
  const bodyRef = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  const contentRef = useRef<HTMLDivElement>(null);
  const [edges, setEdges] = useState({ top: false, bottom: false });
  function syncBody() {
    const el = bodyRef.current;
    if (!el) return;
    if (live && following.current) el.scrollTop = el.scrollHeight;
    const next = thinkingScrollEdges(el);
    setEdges(previous => previous.top === next.top && previous.bottom === next.bottom ? previous : next);
  }
  useLayoutEffect(syncBody, [content, live, open]);
  useLayoutEffect(() => {
    const el = bodyRef.current;
    if (!open || !el) return;
    const observer = new ResizeObserver(syncBody);
    observer.observe(el);
    if (contentRef.current) observer.observe(contentRef.current);
    return () => observer.disconnect();
  }, [open, live]);
  const onScroll = () => {
    const el = bodyRef.current;
    if (!el) return;
    following.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
    const next = thinkingScrollEdges(el);
    setEdges(previous => previous.top === next.top && previous.bottom === next.bottom ? previous : next);
  };
  const mask = edges.top && edges.bottom
    ? 'linear-gradient(to bottom, transparent, black 16px, black calc(100% - 16px), transparent)'
    : edges.top ? 'linear-gradient(to bottom, transparent, black 16px)'
    : edges.bottom ? 'linear-gradient(to bottom, black calc(100% - 16px), transparent)' : undefined;

  const secs =
    startedAt != null ? Math.max(0, Math.round(((endedAt ?? now) - startedAt) / 1000)) : null;
  const dur = secs == null ? null : secs < 1 ? '<1s' : formatElapsed(secs);
  const label = live && !open
    ? 'Thinking…'
    : dur
      ? `Thought for ${dur}`
      : 'Thought';
  const body = content.trim();

  return (
    <div className="min-w-0">
      <button
        type="button"
        onClick={() => {
          touched.current = true;
          setOpen((v) => !v);
        }}
        disabled={!body}
        data-transcript-disclosure
        aria-expanded={open}
        className={cn('group flex w-full min-w-0 items-center gap-1.5 rounded py-1 text-left text-[13px] leading-5 text-muted-foreground transition-colors hover:text-foreground disabled:cursor-default', STREAM_ROW_GUTTER)}
      >
        <span aria-hidden="true" className="inline-flex size-3.5 shrink-0 items-center justify-center"><Brain className="size-3.5" /></span>
        <span className="shrink-0 whitespace-nowrap font-normal"><ActivityShimmer active={live && !open}>{label}</ActivityShimmer></span>
        {!open && live && body && <><span aria-hidden className="shrink-0">·</span><ThinkingPreview content={body} /></>}
        {live && !open && dur && <span className="font-mono text-[11px] text-muted-foreground/70">{dur}</span>}
        {body && (
          <ChevronDown
            strokeWidth={2.5}
            className={cn(
              'size-3 text-muted-foreground/60 transition-transform',
              !open && '-rotate-90',
            )}
          />
        )}
      </button>
      <Collapse open={open && !!body}>
        <div
          ref={bodyRef}
          data-thinking-scroll
          role="region"
          tabIndex={0}
          aria-label="Thinking content"
          onScroll={onScroll}
          onWheel={event => event.stopPropagation()}
          onTouchMove={event => event.stopPropagation()}
          onKeyDown={event => event.stopPropagation()}
          style={{ maskImage: mask }}
          className={cn(
            'thought ml-2 mr-2 mt-1 border-l-2 border-border pl-[18px] text-[13px] leading-relaxed text-muted-foreground',
            'max-h-60 overflow-y-auto overscroll-contain',
          )}
        >
          <div ref={contentRef} className="min-w-0 break-words"><Markdown text={body} onOpenFile={onOpenFile} /></div>
        </div>
      </Collapse>
    </div>
  );
}

function formatElapsed(secs: number): string {
  if (secs < 60) return `${secs}s`;
  return `${Math.floor(secs / 60)}m ${secs % 60}s`;
}
