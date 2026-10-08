import { TranscriptRowContext, TranscriptSessionContext } from './TranscriptDisclosure';
import { transcriptWindow } from '../lib/transcriptWindow';
import { Children, useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from 'react';

type Anchor = { key: string; offset: number; bottom: boolean };
const positions = new Map<string, Anchor>();
function savedPosition(identity: string): Anchor | undefined {
  try { return positions.get(identity) ?? JSON.parse(sessionStorage.getItem(`mira:scroll:${identity}`) ?? 'null') ?? undefined; } catch { return undefined; }
}
export function hasTranscriptPosition(identity: string): boolean { return !!savedPosition(identity) && !savedPosition(identity)?.bottom; }

/** Keep a bounded set of measured transcript rows mounted. Measurements survive
 * streaming updates; browser scroll anchoring preserves the visible row. */
export function VirtualTranscript({ children, pane, identity, onNeedOlder, hasOlder }: { onNeedOlder?: () => void; hasOlder?: boolean; children: ReactNode; pane: RefObject<HTMLDivElement | null>; identity: string }) {
  const rows = Children.toArray(children);
  const root = useRef<HTMLDivElement>(null);
  const pendingRestore = useRef<Anchor | undefined>(undefined);
  const visibleAnchor = useRef<Anchor | undefined>(undefined);
  const previousIdentity = useRef('');
  if (previousIdentity.current !== identity) { previousIdentity.current = identity; pendingRestore.current = savedPosition(identity); visibleAnchor.current = undefined; }
  const offsetsRef = useRef<number[]>([]);
  const keysRef = useRef<string[]>([]);

  const heights = useRef(new Map<string, number>());
  const [viewport, setViewport] = useState({ top: 0, height: 1000 });
  const [, invalidate] = useState(0);
  const keys = rows.map((row, index) => `${identity}:${(row as { key?: string }).key ?? index}`);
  const offsets = [0];
  for (const key of keys) offsets.push(offsets[offsets.length - 1] + (heights.current.get(key) ?? 280));
  offsetsRef.current = offsets; keysRef.current = keys;
  const { start, end } = transcriptWindow(offsets, viewport.top, viewport.height);
  useEffect(() => {
    const scroll = pane.current;
    if (!scroll) return;
    let frame = 0;
    const read = () => {
      frame = 0;
      if (!root.current) return;
      setViewport({ top: scroll.scrollTop - (root.current.getBoundingClientRect().top - scroll.getBoundingClientRect().top + scroll.scrollTop), height: scroll.clientHeight });
    };
    const update = () => { if (!frame) frame = requestAnimationFrame(read); };
    function remember() {
      update();
      if (!root.current || pendingRestore.current) return;
      const boundary = scroll!.getBoundingClientRect().top;
      const row = Array.from(root.current.querySelectorAll<HTMLElement>('[data-virtual-key]')).find(element => element.getBoundingClientRect().bottom > boundary);
      if (!row?.dataset.virtualKey) return;
      const anchor = { key: row.dataset.virtualKey, offset: boundary - row.getBoundingClientRect().top, bottom: scroll!.scrollHeight - scroll!.scrollTop - scroll!.clientHeight < 80 };
      visibleAnchor.current = anchor;
      positions.set(identity, anchor);
      if (positions.size > 100) positions.delete(positions.keys().next().value!);
      try { sessionStorage.setItem(`mira:scroll:${identity}`, JSON.stringify(anchor)); } catch { /* storage is optional */ }
    }
    function jump(event: Event) {
      const id = (event as CustomEvent<string>).detail;
      const nodeIndex = keysRef.current.findIndex(key => key.endsWith(`turn-${id.replace('turn-', '')}`));
      if (nodeIndex < 0 || !root.current) return;
      const top = root.current.getBoundingClientRect().top - scroll!.getBoundingClientRect().top + scroll!.scrollTop;
      pendingRestore.current = { key: keysRef.current[nodeIndex], offset: 0, bottom: false };
      scroll!.scrollTop = top + offsetsRef.current[nodeIndex];
      invalidate(value => value + 1);
      update();
    }
    window.addEventListener('mira:transcript-jump', jump);
    scroll.addEventListener('scroll', remember, { passive: true });
    const resize = new ResizeObserver(update); resize.observe(scroll);
    read();
    return () => { window.removeEventListener('mira:transcript-jump', jump); scroll.removeEventListener('scroll', remember); resize.disconnect(); cancelAnimationFrame(frame); };
  }, [pane, identity, rows.length]);
  useLayoutEffect(() => {
    const anchor = pendingRestore.current;
    const scroll = pane.current;
    if (!anchor || !scroll || !root.current) return;
    if (anchor.bottom) { pendingRestore.current = undefined; scroll.scrollTop = scroll.scrollHeight; return; }
    const index = keys.findIndex(key => key === anchor.key);
    if (index < 0) { if (hasOlder) onNeedOlder?.(); else pendingRestore.current = undefined; return; }
    const origin = root.current.getBoundingClientRect().top - scroll.getBoundingClientRect().top + scroll.scrollTop;
    scroll.scrollTop = origin + offsets[index] + anchor.offset;
    if (heights.current.has(anchor.key)) pendingRestore.current = undefined;
  }, [identity, keys.join('|'), offsets.join('|'), hasOlder]);
  useLayoutEffect(() => {
    if (!root.current) return;
    let frame = 0;
    const observer = new ResizeObserver(items => {
      let changed = false;
      for (const item of items) {
        const key = (item.target as HTMLElement).dataset.virtualKey!;
        const height = item.target.getBoundingClientRect().height;
        if (height > 0 && Math.abs((heights.current.get(key) ?? 280) - height) > 0.5) { heights.current.set(key, height); if (heights.current.size > 10000) heights.current.delete(heights.current.keys().next().value!); changed = true; }
      }
      if (changed && !frame) frame = requestAnimationFrame(() => {
        frame = 0;
        if (!pendingRestore.current && visibleAnchor.current && !visibleAnchor.current.bottom) pendingRestore.current = visibleAnchor.current;
        invalidate(v => v + 1);
      });
    });
    root.current.querySelectorAll('[data-virtual-key]').forEach(row => observer.observe(row));
    return () => { observer.disconnect(); cancelAnimationFrame(frame); };
  }, [start, end, identity, children]);
  return <TranscriptSessionContext.Provider value={identity}><div ref={root}>
    <div aria-hidden style={{ height: offsets[start] }} />
    {rows.slice(start, end).map((row, index) => <div key={keys[start + index]} data-virtual-key={keys[start + index]} className="pb-2"><TranscriptRowContext.Provider value={keys[start + index]}>{row}</TranscriptRowContext.Provider></div>)}
    <div aria-hidden style={{ height: offsets[rows.length] - offsets[end] }} />
  </div></TranscriptSessionContext.Provider>;
}
