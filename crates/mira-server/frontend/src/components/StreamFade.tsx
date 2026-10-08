import { useLayoutEffect, useRef, type ReactNode } from 'react';
import { useReducedMotion } from 'framer-motion';
import { PREF_KEYS, useBoolPref } from '../lib/prefs';

const TAIL_LIMIT = 4096;
/** A bounded prose tail: never wrap or replace React's text nodes. */
function proseTail(root: HTMLElement) {
  const nodes: Text[] = [];
  let size = 0;
  const visit = (node: Node) => {
    if (size >= TAIL_LIMIT) return;
    if (node instanceof Element && node.matches('pre, code, button, svg, [aria-hidden="true"]')) return;
    if (node instanceof Text) { nodes.unshift(node); size += node.length; return; }
    for (let child = node.lastChild; child && size < TAIL_LIMIT; child = child.previousSibling) visit(child);
  };
  visit(root);
  const all = nodes.map(node => node.data).join('');
  return { nodes, text: all.slice(-TAIL_LIMIT), skipped: Math.max(0, all.length - TAIL_LIMIT) };
}

/** Fade only new glyph rectangles. CSS opacity animations are bounded to
 * eight overlays; no per-character spans, timers, or React animation state. */
export function StreamFade({ children, streaming = false, revision }: { children: ReactNode; streaming?: boolean; revision: string }) {
  const root = useRef<HTMLDivElement>(null);
  const layer = useRef<HTMLDivElement>(null);
  const previous = useRef<string | null>(null);
  const systemReducedMotion = useReducedMotion();
  const [reducedMotion] = useBoolPref(PREF_KEYS.reduceMotion, false);
  useLayoutEffect(() => {
    const element = root.current;
    const overlay = layer.current;
    if (!element || !overlay) return;
    const tail = proseTail(element);
    const prior = previous.current;
    previous.current = tail.text;
    if (!streaming || systemReducedMotion || reducedMotion) { overlay.replaceChildren(); return; }
    let added = prior === null ? Math.min(128, tail.text.length) : 0;
    if (prior !== null && tail.text.startsWith(prior)) added = tail.text.length - prior.length;
    else if (prior) {
      const anchor = prior.slice(-256);
      const offset = tail.text.lastIndexOf(anchor);
      if (offset >= 0) added = tail.text.length - offset - anchor.length;
    }
    // Corrections and markdown rewrites must not replay the whole answer.
    if (added <= 0 || added > 512) return;
    const from = tail.skipped + tail.text.length - added;
    let offset = 0;
    let start: Text | undefined;
    let startOffset = 0;
    for (const node of tail.nodes) {
      if (offset + node.length > from) { start = node; startOffset = from - offset; break; }
      offset += node.length;
    }
    const end = tail.nodes.at(-1);
    if (!start || !end) return;
    const rectangles: DOMRect[] = [];
    let started = false;
    for (const node of tail.nodes) {
      if (node === start) started = true;
      if (!started) continue;
      const range = document.createRange();
      range.setStart(node, node === start ? startOffset : 0); range.setEnd(node, node.length);
      rectangles.push(...Array.from(range.getClientRects()));
      if (rectangles.length >= 8) break;
    }
    const bounds = element.getBoundingClientRect();
    let ancestor: Element | null = element;
    let background = '';
    while (ancestor) {
      const color = getComputedStyle(ancestor).backgroundColor;
      if (color !== 'transparent' && color !== 'rgba(0, 0, 0, 0)') { background = color; break; }
      ancestor = ancestor.parentElement;
    }
    for (const rect of rectangles.slice(0, 8)) {
      if (!rect.width || !rect.height) continue;
      const mask = document.createElement('span');
      mask.className = 'stream-text-reveal';
      Object.assign(mask.style, { left: `${rect.left - bounds.left}px`, top: `${rect.top - bounds.top}px`, width: `${rect.width}px`, height: `${rect.height}px`, background: background || 'hsl(var(--background))' });
      mask.addEventListener('animationend', () => mask.remove(), { once: true });
      overlay.append(mask);
    }
    while (overlay.childElementCount > 8) overlay.firstElementChild?.remove();
  }, [revision, streaming, systemReducedMotion, reducedMotion]);
  useLayoutEffect(() => () => { layer.current?.replaceChildren(); }, []);
  return <div ref={root} className="relative">{children}<div ref={layer} aria-hidden="true" className="pointer-events-none absolute inset-0 overflow-visible" /></div>;
}
