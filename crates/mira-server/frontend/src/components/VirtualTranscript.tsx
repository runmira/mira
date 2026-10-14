import { Children, useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from 'react';
import { anchorTop, transcriptAnchor } from '../lib/transcriptAnchor';
import { type Anchor, positions, savedPosition } from '../lib/transcriptPosition';
import { transcriptWindow } from '../lib/transcriptWindow';
import { TranscriptFollowContext, TranscriptRowContext, TranscriptSessionContext } from './TranscriptDisclosure';

/** One scroll owner: virtual measurements, reading anchors and end following. */
export function VirtualTranscript({ children, pane, identity, onNeedOlder, hasOlder, follow, lastInput, readingAnchor }: {
  children: ReactNode;
  pane: RefObject<HTMLDivElement | null>;
  identity: string;
  onNeedOlder?: () => void;
  hasOlder?: boolean;
  follow?: { current: boolean };
  lastInput?: { current: number };
  readingAnchor?: { current: number | null };
}) {
  const rows = Children.toArray(children);
  // Ancestor host refs attach after child layout effects on first mount.
  // Re-run ownership effects once the actual scroll pane is available.
  const [scrollElement, setScrollElement] = useState<HTMLDivElement | null>(null);
  useEffect(() => { setScrollElement(pane.current); }, [pane, identity]);
  const root = useRef<HTMLDivElement>(null);
  const heights = useRef(new Map<string, number>());
  const committed = useRef({ identity: '', keys: [] as string[], offsets: [0] });
  const restore = useRef<Anchor | undefined>(undefined);
  const disclosure = useRef<{ element: HTMLElement; top: number; end: boolean } | undefined>(undefined);
  const [viewport, setViewport] = useState({ top: 0, height: 1000 });
  const [, invalidate] = useState(0);
  const keys = rows.map((row, index) => `${identity}:${(row as { key?: string }).key ?? index}`);
  const offsets = [0];
  for (const key of keys) offsets.push(offsets.at(-1)! + (heights.current.get(key) ?? 280));
  const { start, end } = transcriptWindow(offsets, viewport.top, viewport.height);
  const latest = useRef({ keys, offsets, follow, lastInput, onNeedOlder, hasOlder });
  latest.current = { keys, offsets, follow, lastInput, onNeedOlder, hasOlder };

  function origin() {
    const scroll = pane.current!;
    return root.current!.getBoundingClientRect().top - scroll.getBoundingClientRect().top + scroll.scrollTop;
  }
  function readViewport() {
    const scroll = scrollElement;
    if (!scroll || !root.current) return;
    const top = scroll.scrollTop - origin(), height = scroll.clientHeight;
    setViewport(previous => previous.top === top && previous.height === height ? previous : { top, height });
  }
  function following() {
    return !!latest.current.follow?.current && Date.now() - (latest.current.lastInput?.current ?? 0) >= 700;
  }

  // Capture from the committed layout before replacing spacer estimates. This
  // also preserves stable row IDs when older history is prepended.
  useLayoutEffect(() => {
    const scroll = scrollElement;
    if (!scroll || !root.current) return;
    const old = committed.current;
    const switched = old.identity !== identity;
    const changed = old.keys.join('|') !== keys.join('|') || old.offsets.join('|') !== offsets.join('|');
    const anchor = switched ? savedPosition(identity) : restore.current ??
      (changed && !following() ? transcriptAnchor(old.keys, old.offsets, scroll.scrollTop - origin()) : undefined);
    restore.current = anchor;
    committed.current = { identity, keys, offsets };
    if (readingAnchor?.current != null) {
      scroll.scrollTop = readingAnchor.current;
      readingAnchor.current = null;
      restore.current = undefined;
    } else if (anchor && !anchor.bottom) {
      const top = anchorTop(anchor, keys, offsets);
      if (top !== undefined) {
        const nextTop = origin() + top;
        if (Math.abs(scroll.scrollTop - nextTop) > 0.5) scroll.scrollTop = nextTop;
        if (heights.current.has(anchor.key)) restore.current = undefined;
      } else if (hasOlder) onNeedOlder?.();
      else restore.current = undefined;
    } else if (anchor?.bottom || following()) {
      scroll.scrollTop = scroll.scrollHeight;
      restore.current = undefined;
    }
    const pinned = disclosure.current;
    if (pinned?.element.isConnected) {
      if (pinned.end && following()) scroll.scrollTop = scroll.scrollHeight;
      else { const delta = pinned.element.getBoundingClientRect().top - pinned.top;
        if (Math.abs(delta) > 0.5) scroll.scrollTop += delta; }
    }
    readViewport();
  });

  // Measure newly mounted rows before paint, and asynchronously for images/tool
  // output. The observer depends on mounted row identity, never token content.
  useLayoutEffect(() => {
    const scroll = scrollElement;
    if (!root.current || !scroll) return;
    function measure() {
      const old = committed.current;
      const anchor = transcriptAnchor(old.keys, old.offsets, scroll!.scrollTop - origin());
      let changed = false;
      root.current?.querySelectorAll<HTMLElement>('[data-virtual-key]').forEach(row => {
        const key = row.dataset.virtualKey!, height = row.getBoundingClientRect().height;
        if (height > 0 && (!heights.current.has(key) || Math.abs(heights.current.get(key)! - height) > 0.5)) {
          heights.current.set(key, height);
          if (heights.current.size > 10000) heights.current.delete(heights.current.keys().next().value!);
          changed = true;
        }
      });
      if (changed) {
        if (!restore.current && !following()) restore.current = anchor;
        invalidate(value => value + 1);
      }
    }
    measure();
    const observer = new ResizeObserver(measure);
    root.current.querySelectorAll('[data-virtual-key]').forEach(row => observer.observe(row));
    return () => observer.disconnect();
  }, [scrollElement, identity, start, end, keys.join('|')]);

  // Streaming can resize existing rows during a React commit without creating
  // a new observer. ResizeObserver reports those changes before the next paint.
  useLayoutEffect(() => {
    const scroll = scrollElement;
    if (!scroll) return;
    let settle = 0;
    function remember() {
      readViewport();
      if (!root.current || restore.current) return;
      const model = committed.current;
      const anchor = transcriptAnchor(model.keys, model.offsets, scroll!.scrollTop - origin());
      if (!anchor) return;
      anchor.bottom = !!latest.current.follow?.current && scroll!.scrollHeight - scroll!.scrollTop - scroll!.clientHeight < 80;
      positions.set(identity, anchor);
      if (positions.size > 100) positions.delete(positions.keys().next().value!);
      try { sessionStorage.setItem(`mira:scroll:${identity}`, JSON.stringify(anchor)); } catch { /* optional */ }
    }
    function input(event: Event) {
      if ((event.target as Element)?.closest?.('[data-thinking-scroll], [data-live-output]')) return;
      disclosure.current = undefined;
      // A new gesture wins over unfinished restore/measurement work.
      restore.current = undefined;
    }
    function jump(event: Event) {
      const id = (event as CustomEvent<string>).detail;
      const model = committed.current;
      const index = model.keys.findIndex(key => key.endsWith(`turn-${id.replace('turn-', '')}`));
      if (index < 0 || !root.current) return;
      if (latest.current.follow) latest.current.follow.current = false;
      restore.current = { key: model.keys[index], offset: 0, bottom: false };
      scroll!.scrollTop = origin() + model.offsets[index];
      invalidate(value => value + 1);
    }
    function toggle(event: Event) {
      const target = (event.target as HTMLElement).closest<HTMLElement>('[aria-expanded]');
      if (!target || !root.current?.contains(target)) return;
      const atEnd = scroll!.scrollHeight - scroll!.scrollTop - scroll!.clientHeight < 2;
      disclosure.current = { element: target, top: target.getBoundingClientRect().top, end: atEnd };
      if (!atEnd && latest.current.follow) latest.current.follow.current = false;
      clearTimeout(settle);
      settle = window.setTimeout(() => { disclosure.current = undefined; }, 700);
    }
    scroll.addEventListener('scroll', remember, { passive: true });
    scroll.addEventListener('wheel', input, { passive: true });
    scroll.addEventListener('touchmove', input, { passive: true });
    scroll.addEventListener('keydown', input);
    scroll.addEventListener('pointerdown', input, { passive: true });
    root.current?.addEventListener('click', toggle, true);
    window.addEventListener('mira:transcript-jump', jump);
    const observer = new ResizeObserver(readViewport);
    observer.observe(scroll);
    return () => {
      clearTimeout(settle);
      observer.disconnect();
      scroll.removeEventListener('scroll', remember);
      scroll.removeEventListener('wheel', input);
      scroll.removeEventListener('touchmove', input);
      scroll.removeEventListener('keydown', input);
      scroll.removeEventListener('pointerdown', input);
      root.current?.removeEventListener('click', toggle, true);
      window.removeEventListener('mira:transcript-jump', jump);
    };
  }, [scrollElement, identity]);

  return <TranscriptSessionContext.Provider value={identity}><TranscriptFollowContext.Provider value={follow}>
    <div ref={root}>
      <div aria-hidden style={{ height: offsets[start], overflowAnchor: 'none' }} />
      {rows.slice(start, end).map((row, index) => <div key={keys[start + index]} data-virtual-key={keys[start + index]} className="pb-2">
        <TranscriptRowContext.Provider value={keys[start + index]}>{row}</TranscriptRowContext.Provider>
      </div>)}
      <div aria-hidden style={{ height: offsets[rows.length] - offsets[end], overflowAnchor: 'none' }} />
    </div>
  </TranscriptFollowContext.Provider></TranscriptSessionContext.Provider>;
}
export { hasTranscriptPosition } from '../lib/transcriptPosition';
