import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ArrowUp, LoaderCircle, MessageSquare } from 'lucide-react';
import { cn } from '@/lib/utils';
import { annotationToComposer, isArea, type Mark } from '@/lib/browserAnnotation';

/** Where the agent last acted in the page, in CSS pixels. `seq` changes on
 *  every action so a second click on the same spot still ripples. */
export type AgentPointer = { x: number; y: number; action: string; seq: number };

/**
 * The live frame, sized to the largest box of the page's aspect ratio that
 * fits the pane, with layers stacked exactly over the page: anything placed
 * in page percentages lands on the pixel it means.
 */
export function FittedFrame({ w, h, children }: { w: number; h: number; children: React.ReactNode }) {
  const outer = useRef<HTMLDivElement | null>(null);
  const [box, setBox] = useState<{ width: number; height: number } | null>(null);
  useLayoutEffect(() => {
    const el = outer.current;
    if (!el || !w || !h) return;
    const fit = () => {
      const r = el.getBoundingClientRect();
      const scale = Math.min(r.width / w, r.height / h);
      setBox({ width: w * scale, height: h * scale });
    };
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    return () => ro.disconnect();
  }, [w, h]);
  return (
    <div ref={outer} className="flex size-full items-center justify-center">
      <div className="relative" style={box ?? { width: '100%', height: '100%' }}>
        {children}
      </div>
    </div>
  );
}

const ACTION_WORDS: Record<string, string> = {
  click: 'clicking', type: 'typing', hover: 'looking', scroll: 'scrolling', drag: 'dragging',
};

/**
 * The agent's cursor: a dark arrow on a soft blue glow that glides to each
 * point the agent acts on, with a tag saying what it's doing. The glow
 * breathes while it works and flares on a click; it all fades once the
 * agent has been still for a while.
 */
export function AgentCursor({ pointer, w, h }: { pointer: AgentPointer | null; w: number; h: number }) {
  const [idle, setIdle] = useState(true);
  useEffect(() => {
    if (!pointer) return;
    setIdle(false);
    const t = setTimeout(() => setIdle(true), 6000);
    return () => clearTimeout(t);
  }, [pointer]);
  if (!pointer || !w || !h) return null;
  const pressing = pointer.action === 'click' || pointer.action === 'type' || pointer.action === 'drag';
  return (
    <div
      aria-hidden
      className="pointer-events-none absolute left-0 top-0 z-20 transition-[left,top,opacity] duration-[420ms] ease-[cubic-bezier(0.22,1,0.36,1)]"
      style={{ left: `${(pointer.x / w) * 100}%`, top: `${(pointer.y / h) * 100}%`, opacity: idle ? 0 : 1 }}
    >
      {/* The glow, centred on the tip. */}
      <span className="agent-cursor-glow absolute -left-[34px] -top-[34px] size-[68px] rounded-full bg-[radial-gradient(circle,rgba(47,107,255,0.42)_0%,rgba(47,107,255,0.16)_40%,rgba(47,107,255,0)_70%)]" />
      {pressing && (
        <span
          key={pointer.seq}
          className="agent-cursor-ripple absolute -left-[16px] -top-[16px] size-8 rounded-full border-[1.5px] border-mira-blue/80 bg-mira-blue/10"
        />
      )}
      <svg
        key={`c${pointer.seq}`}
        width="24"
        height="24"
        viewBox="0 0 24 24"
        className={cn('relative -ml-[3px] -mt-[2px] drop-shadow-[0_3px_6px_rgba(10,20,60,0.35)]', pressing && 'agent-cursor-press')}
      >
        <defs>
          <linearGradient id="agent-cursor-fill" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0" stopColor="#2a2f3a" />
            <stop offset="1" stopColor="#07090d" />
          </linearGradient>
        </defs>
        <path
          d="M4.2 2.6 19 13.1a.7.7 0 0 1-.3 1.25l-5.7 1.06-3.06 4.98a.7.7 0 0 1-1.27-.2L3.06 3.35a.7.7 0 0 1 1.14-.75Z"
          fill="url(#agent-cursor-fill)"
          stroke="#fff"
          strokeWidth="1.6"
          strokeLinejoin="round"
        />
      </svg>
      <span className="absolute left-[18px] top-[20px] flex items-center gap-1 whitespace-nowrap rounded-full bg-mira-blue py-0.5 pl-1.5 pr-2 text-[10.5px] font-semibold text-white shadow-[0_4px_12px_-2px_rgba(47,107,255,0.55)]">
        <span className="size-1.5 rounded-full bg-white/90" />
        Mira{ACTION_WORDS[pointer.action] ? ` · ${ACTION_WORDS[pointer.action]}` : ''}
      </span>
    </div>
  );
}

type Note = Mark & { comment: string };

/** A pin: a blue drop whose point sits on the marked spot. */
function Pin({ x, y, w, h, title, pop }: { x: number; y: number; w: number; h: number; title?: string; pop?: boolean }) {
  return (
    <span
      title={title}
      className={cn(
        'absolute size-[18px] rounded-[2px_50%_50%_50%] bg-mira-blue shadow-[0_2px_6px_rgba(0,0,0,0.3)]',
        pop && 'annotation-pin',
        title ? 'pointer-events-auto cursor-default' : 'pointer-events-none',
      )}
      style={{ left: `${(x / w) * 100}%`, top: `${(y / h) * 100}%`, transformOrigin: '0 0' }}
    />
  );
}

/** The hint under the Annotate button while the mode is new. */
export function AnnotateHint() {
  return (
    <div
      role="status"
      className="pointer-events-none absolute -left-14 top-[calc(100%+8px)] z-50 w-[280px] rounded-xl bg-[#1f1f22]/95 px-3.5 py-2.5 text-[12.5px] leading-snug text-white/90 shadow-[0_12px_30px_-8px_rgba(0,0,0,0.45)] backdrop-blur"
    >
      Click anywhere to annotate, or drag to select an area. Hold{' '}
      <kbd className="rounded-md bg-white/15 px-1.5 py-px font-sans text-[11.5px] text-white">Space</kbd> to
      temporarily use the webpage.
    </div>
  );
}

/**
 * Annotation mode over the live frame. Click to pin a note to a spot, drag
 * to mark an area; each note goes to the composer as a card. Holding Space
 * lets pointer input through to the page.
 */
export function AnnotationLayer({ frame, url, onExit, onMark }: {
  frame: { src: string; w: number; h: number };
  url: string;
  onExit: () => void;
  /** The user started a mark (the hint can go). */
  onMark: () => void;
}) {
  const layer = useRef<HTMLDivElement | null>(null);
  const input = useRef<HTMLInputElement | null>(null);
  const [notes, setNotes] = useState<Note[]>([]);
  const [draft, setDraft] = useState<Mark | null>(null);
  const [drag, setDrag] = useState<{ x0: number; y0: number; x: number; y: number; moved: boolean } | null>(null);
  const [comment, setComment] = useState('');
  const [sending, setSending] = useState(false);
  const [passThrough, setPassThrough] = useState(false);

  // Space held = use the page; Escape drops the draft, then leaves the mode.
  useEffect(() => {
    const typing = (e: KeyboardEvent) => e.target instanceof HTMLElement && (e.target.isContentEditable || /^(INPUT|TEXTAREA)$/.test(e.target.tagName));
    const down = (e: KeyboardEvent) => {
      if (e.key === ' ' && !typing(e)) { e.preventDefault(); setPassThrough(true); }
      if (e.key === 'Escape' && !typing(e)) { if (draft) setDraft(null); else onExit(); }
    };
    const up = (e: KeyboardEvent) => { if (e.key === ' ') setPassThrough(false); };
    const blur = () => setPassThrough(false);
    window.addEventListener('keydown', down);
    window.addEventListener('keyup', up);
    window.addEventListener('blur', blur);
    return () => { window.removeEventListener('keydown', down); window.removeEventListener('keyup', up); window.removeEventListener('blur', blur); };
  }, [draft, onExit]);

  // Focus the comment without scrolling: the pane must not shift.
  useEffect(() => { if (draft) input.current?.focus({ preventScroll: true }); }, [draft]);

  /** Pointer position in page CSS pixels. */
  const toPage = (e: React.PointerEvent) => {
    const r = layer.current!.getBoundingClientRect();
    return { x: ((e.clientX - r.left) / r.width) * frame.w, y: ((e.clientY - r.top) / r.height) * frame.h };
  };
  const pct = (v: number, of: number) => `${(v / of) * 100}%`;

  async function submit() {
    if (!draft || !comment.trim() || sending) return;
    setSending(true);
    try {
      await annotationToComposer({ src: frame.src, frame, url, mark: draft, comment });
      setNotes((all) => [...all, { ...draft, comment: comment.trim() }]);
      setDraft(null);
      setComment('');
    } finally {
      setSending(false);
    }
  }

  const live = drag?.moved
    ? { x: Math.min(drag.x0, drag.x), y: Math.min(drag.y0, drag.y), w: Math.abs(drag.x - drag.x0), h: Math.abs(drag.y - drag.y0) }
    : null;
  const outlines = [...notes, ...(draft ? [draft] : []), ...(live ? [live] : [])].filter(isArea);
  // The comment pill sits just under the mark, starting at its left edge,
  // and always inside the frame (an input focused off the edge would
  // scroll the page view sideways).
  const pill = 'min(340px, 100% - 16px)';
  const box = draft && {
    left: `clamp(8px, ${pct(draft.x, frame.w)}, calc(100% - ${pill} - 8px))`,
    top: `clamp(8px, calc(${pct(draft.y + (draft.h ?? 0), frame.h)} + ${isArea(draft) ? 10 : 26}px), calc(100% - 52px))`,
  };

  return (
    <div
      ref={layer}
      className={cn('absolute inset-0 z-10 select-none', passThrough ? 'pointer-events-none' : 'cursor-crosshair')}
      onPointerDown={(e) => {
        if (e.button !== 0 || e.target !== e.currentTarget) return;
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
        const p = toPage(e);
        onMark();
        setDraft(null);
        setComment('');
        setDrag({ x0: p.x, y0: p.y, x: p.x, y: p.y, moved: false });
      }}
      onPointerMove={(e) => {
        if (!drag) return;
        const p = toPage(e);
        const r = layer.current!.getBoundingClientRect();
        const far = Math.hypot(((p.x - drag.x0) / frame.w) * r.width, ((p.y - drag.y0) / frame.h) * r.height) > 6;
        setDrag({ ...drag, x: p.x, y: p.y, moved: drag.moved || far });
      }}
      onPointerUp={() => {
        if (!drag) return;
        setDraft(live ?? { x: drag.x0, y: drag.y0 });
        setDrag(null);
      }}
      onKeyDown={(e) => e.stopPropagation()}
    >
      {outlines.map((m, i) => (
        <div
          key={`o${i}`}
          className="pointer-events-none absolute border-[1.5px] border-mira-blue"
          style={{ left: pct(m.x, frame.w), top: pct(m.y, frame.h), width: pct(m.w ?? 0, frame.w), height: pct(m.h ?? 0, frame.h) }}
        />
      ))}
      {notes.map((m, i) => (
        <Pin key={`p${i}`} x={m.x} y={m.y} w={frame.w} h={frame.h} title={m.comment} />
      ))}
      {draft && <Pin x={draft.x} y={draft.y} w={frame.w} h={frame.h} pop />}

      {draft && box && (
        <form
          className="absolute z-20 flex items-center gap-2 rounded-full border border-black/[0.06] bg-white py-1 pl-3.5 pr-1 text-[#1a1a1a] shadow-[0_10px_30px_-6px_rgba(0,0,0,0.28),0_1px_3px_rgba(0,0,0,0.08)] dark:border-white/10 dark:bg-[#2a2a2d] dark:text-white"
          style={{ ...box, width: pill }}
          onPointerDown={(e) => e.stopPropagation()}
          onSubmit={(e) => { e.preventDefault(); void submit(); }}
        >
          <MessageSquare className="size-3.5 shrink-0 opacity-45" aria-hidden />
          <input
            ref={input}
            value={comment}
            onChange={(e) => setComment(e.target.value)}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === 'Escape') { setDraft(null); setComment(''); }
            }}
            placeholder="Add a comment..."
            className="min-w-0 flex-1 bg-transparent py-1.5 text-[13px] outline-none placeholder:text-current placeholder:opacity-40"
          />
          <button
            type="submit"
            title="Add to message"
            disabled={!comment.trim() || sending}
            className="flex size-7 shrink-0 items-center justify-center rounded-full bg-mira-blue text-white transition-opacity disabled:bg-black/10 disabled:text-black/40 dark:disabled:bg-white/10 dark:disabled:text-white/40"
          >
            {sending ? <LoaderCircle className="size-3.5 animate-spin" /> : <ArrowUp className="size-3.5" />}
          </button>
        </form>
      )}
    </div>
  );
}
