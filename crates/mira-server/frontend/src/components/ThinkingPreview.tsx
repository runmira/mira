import { useLayoutEffect, useRef, useState } from 'react';
import { useReducedMotion } from 'framer-motion';
import { PREF_KEYS, useBoolPref } from '../lib/prefs';
import { usePacedSummary } from '../lib/usePacedSummary';
import { thinkingPreview } from '../lib/thinkingPreview';

/** Last-line preview follows new tokens horizontally without widening the row. */
export function ThinkingPreview({ content }: { content: string }) {
  const next = thinkingPreview(content);
  const displayed = usePacedSummary({ key: next.key, version: next.text, value: next }, true);
  const summary = displayed.value;
  const viewport = useRef<HTMLSpanElement>(null);
  const text = useRef<HTMLSpanElement>(null);
  const [overflow, setOverflow] = useState(false);
  const reduced = useReducedMotion();
  const [preferReduced] = useBoolPref(PREF_KEYS.reduceMotion, false);
  function sync() {
    const el = viewport.current;
    if (!el) return;
    el.scrollLeft = el.scrollWidth;
    const next = el.scrollWidth > el.clientWidth + 1;
    setOverflow(previous => previous === next ? previous : next);
  }
  useLayoutEffect(sync, [summary.text]);
  useLayoutEffect(() => {
    if (!viewport.current) return;
    const observer = new ResizeObserver(sync);
    observer.observe(viewport.current);
    // A persistent outer span receives size changes as keyed lines roll in.
    if (text.current) observer.observe(text.current);
    return () => observer.disconnect();
  }, []);
  return <span ref={viewport} data-thinking-preview
    className="min-w-0 flex-1 overflow-hidden whitespace-nowrap text-muted-foreground/70"
    style={overflow ? { maskImage: 'linear-gradient(to right, transparent, black 16px, black calc(100% - 16px), transparent)' } : undefined}>
    <span ref={text} className="inline-block min-w-max h-5 overflow-hidden align-middle">
      <span key={summary.key} className="thinking-line-enter inline-block" style={reduced || preferReduced ? { animation: 'none' } : undefined}>
        {summary.text}
      </span>
    </span>
  </span>;
}
