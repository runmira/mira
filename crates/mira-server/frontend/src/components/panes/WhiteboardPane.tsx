import { useCallback, useEffect, useRef, useState } from 'react';
import { Eraser, Pencil, Redo2, Send, Trash2, Undo2 } from 'lucide-react';
import { cn } from '@/lib/utils';

/** One continuous pen-down → pen-up gesture. */
type Stroke = {
  color: string;
  width: number;
  /** Flat [x0,y0,x1,y1,…] in CSS pixels relative to the canvas. */
  points: number[];
};

type Tool = 'pen' | 'eraser';

const PALETTE = [
  '#e4e4e7', // near-white — the default on a dark canvas
  '#f7768e', // error red
  '#ffb86b', // amber
  '#7dcfff', // cyan
  '#a6d189', // green
  '#7aa2f7', // blue
  '#bb9af7', // purple
];

const WIDTHS = [2, 4, 8];

/** Index-addressed base stroke widths — the width buttons map 1:1. */
const PALETTE_BASE = WIDTHS;

/** Eraser width multiplier. A true destination-out erase would destroy the
 *  strokes, which breaks undo; painting the background color instead keeps
 *  the stroke model intact. */
const ERASER_SCALE = 6;

export function WhiteboardPane({
  onSendToChat,
}: {
  /** Receives a PNG data URL of the current drawing. The caller turns it
   *  into a composer attachment so the model can actually see it — sending
   *  coordinates as text would be useless to it. */
  onSendToChat: (pngDataUrl: string) => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const wrapRef = useRef<HTMLDivElement | null>(null);

  const [tool, setTool] = useState<Tool>('pen');
  const [color, setColor] = useState(PALETTE[0]);
  const [widthIdx, setWidthIdx] = useState(1);
  /** Bumped to force a redraw of the committed strokes after undo/clear. */
  const [version, setVersion] = useState(0);
  const [empty, setEmpty] = useState(true);

  const strokes = useRef<Stroke[]>([]);
  const redoStack = useRef<Stroke[]>([]);
  const live = useRef<Stroke | null>(null);
  const drawing = useRef(false);

  const bg = '#1a1c20';

  /** Redraw everything: committed strokes, then the in-progress one. */
  const repaint = useCallback(() => {
    const cv = canvasRef.current;
    if (!cv) return;
    const ctx = cv.getContext('2d');
    if (!ctx) return;

    ctx.clearRect(0, 0, cv.width, cv.height);

    const draw = (s: Stroke, scale: number) => {
      const pts = s.points;
      if (pts.length < 2) return;
      ctx.save();
      ctx.lineCap = 'round';
      ctx.lineJoin = 'round';
      ctx.lineWidth = s.width * scale;
      ctx.strokeStyle = s.color;
      ctx.beginPath();
      ctx.moveTo(pts[0], pts[1]);
      // Midpoint quadratic smoothing — raw polyline segments look jagged at
      // low pointer-event rates, which is most of them on a trackpad.
      for (let i = 2; i < pts.length - 2; i += 2) {
        const mx = (pts[i] + pts[i + 2]) / 2;
        const my = (pts[i + 1] + pts[i + 3]) / 2;
        ctx.quadraticCurveTo(pts[i], pts[i + 1], mx, my);
      }
      ctx.lineTo(pts[pts.length - 2], pts[pts.length - 1]);
      ctx.stroke();
      ctx.restore();
    };

    const scale = window.devicePixelRatio || 1;
    for (const s of strokes.current) draw(s, scale);
    if (live.current) draw(live.current, scale);
  }, []);

  /** Match the backing store to the element's CSS size × DPR. Returns true
   *  when the size changed, which is the signal to repaint. */
  const syncSize = useCallback((): boolean => {
    const cv = canvasRef.current;
    const wrap = wrapRef.current;
    if (!cv || !wrap) return false;
    const dpr = window.devicePixelRatio || 1;
    const rect = wrap.getBoundingClientRect();
    const w = Math.max(1, Math.floor(rect.width));
    const h = Math.max(1, Math.floor(rect.height));
    if (cv.width === w * dpr && cv.height === h * dpr) return false;
    cv.width = w * dpr;
    cv.height = h * dpr;
    cv.style.width = `${w}px`;
    cv.style.height = `${h}px`;
    return true;
  }, []);

  // Initial + resize paint. The panel is resizable and the tab strip can
  // open/close it, so we observe the wrapper rather than trusting a mount
  // time measurement.
  useEffect(() => {
    const wrap = wrapRef.current;
    if (!wrap) return;
    const ro = new ResizeObserver(() => {
      if (syncSize()) repaint();
    });
    ro.observe(wrap);
    syncSize();
    repaint();
    return () => ro.disconnect();
  }, [syncSize, repaint]);

  // Committed strokes only change through undo/redo/clear.
  useEffect(() => {
    repaint();
  }, [version, repaint]);

  function pointFromEvent(e: React.PointerEvent<HTMLCanvasElement>): [number, number] {
    const cv = canvasRef.current!;
    const rect = cv.getBoundingClientRect();
    return [e.clientX - rect.left, e.clientY - rect.top];
  }

  function onPointerDown(e: React.PointerEvent<HTMLCanvasElement>) {
    if (e.button !== 0 && e.pointerType === 'mouse') return;
    e.currentTarget.setPointerCapture(e.pointerId);
    drawing.current = true;
    redoStack.current = [];
    const [x, y] = pointFromEvent(e);
    live.current = {
      color: tool === 'eraser' ? bg : color,
      width: PALETTE_BASE[widthIdx] * (tool === 'eraser' ? ERASER_SCALE : 1),
      points: [x, y],
    };
    repaint();
  }

  function onPointerMove(e: React.PointerEvent<HTMLCanvasElement>) {
    if (!drawing.current || !live.current) return;
    const [x, y] = pointFromEvent(e);
    const last = live.current.points;
    // Drop sub-pixel jitter so the quadratic smoothing has something to
    // work with and the point array doesn't balloon.
    const dx = x - last[last.length - 2];
    const dy = y - last[last.length - 1];
    if (dx * dx + dy * dy < 0.5) return;
    last.push(x, y);
    repaint();
  }

  function onPointerUp() {
    if (!drawing.current) return;
    drawing.current = false;
    const s = live.current;
    live.current = null;
    if (s && s.points.length >= 2) {
      strokes.current = [...strokes.current, s];
      setEmpty(false);
    }
    setVersion((v) => v + 1);
  }

  function undo() {
    if (strokes.current.length === 0) return;
    const last = strokes.current[strokes.current.length - 1];
    strokes.current = strokes.current.slice(0, -1);
    redoStack.current = [...redoStack.current, last];
    setEmpty(strokes.current.length === 0);
    setVersion((v) => v + 1);
  }

  function redo() {
    const s = redoStack.current[redoStack.current.length - 1];
    if (!s) return;
    redoStack.current = redoStack.current.slice(0, -1);
    strokes.current = [...strokes.current, s];
    setEmpty(false);
    setVersion((v) => v + 1);
  }

  function clear() {
    strokes.current = [];
    redoStack.current = [];
    setEmpty(true);
    setVersion((v) => v + 1);
  }

  function send() {
    const cv = canvasRef.current;
    if (!cv || strokes.current.length === 0) return;
    // Composite onto an opaque background: the canvas backing store is
    // transparent where nothing was drawn, and a transparent PNG would
    // show up as a black rectangle in most viewers.
    const out = document.createElement('canvas');
    out.width = cv.width;
    out.height = cv.height;
    const ctx = out.getContext('2d');
    if (!ctx) return;
    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, out.width, out.height);
    ctx.drawImage(cv, 0, 0);
    onSendToChat(out.toDataURL('image/png'));
  }

  const baseWidth = PALETTE_BASE[widthIdx];

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* toolbar */}
      <div className="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-border/60 px-2.5 py-1.5">
        <ToolBtn active={tool === 'pen'} onClick={() => setTool('pen')} title="Pen">
          <Pencil className="size-3.5" />
        </ToolBtn>
        <ToolBtn
          active={tool === 'eraser'}
          onClick={() => setTool('eraser')}
          title="Eraser"
        >
          <Eraser className="size-3.5" />
        </ToolBtn>

        <span className="mx-1 h-4 w-px bg-border" />

        {PALETTE.map((c) => (
          <button
            key={c}
            type="button"
            title={c}
            onClick={() => {
              setColor(c);
              setTool('pen');
            }}
            className={cn(
              'size-4 shrink-0 rounded-full transition-transform',
              color === c && tool === 'pen'
                ? 'ring-2 ring-foreground/70 ring-offset-1 ring-offset-background'
                : 'hover:scale-110',
            )}
            style={{ backgroundColor: c }}
          >
            <span className="sr-only">{c}</span>
          </button>
        ))}

        <span className="mx-1 h-4 w-px bg-border" />

        {WIDTHS.map((w, i) => (
          <button
            key={w}
            type="button"
            title={`${w}px`}
            onClick={() => setWidthIdx(i)}
            className={cn(
              'flex size-5 items-center justify-center rounded transition-colors',
              widthIdx === i ? 'bg-fg/[0.1]' : 'hover:bg-fg/[0.05]',
            )}
          >
            <span
              className="rounded-full bg-foreground/80"
              style={{ width: w + 2, height: w + 2 }}
            />
          </button>
        ))}

        <span className="flex-1" />

        <ToolBtn onClick={undo} title="Undo" disabled={strokes.current.length === 0}>
          <Undo2 className="size-3.5" />
        </ToolBtn>
        <ToolBtn onClick={redo} title="Redo" disabled={redoStack.current.length === 0}>
          <Redo2 className="size-3.5" />
        </ToolBtn>
        <ToolBtn onClick={clear} title="Clear" disabled={empty}>
          <Trash2 className="size-3.5" />
        </ToolBtn>
        <button
          type="button"
          onClick={send}
          disabled={empty}
          className="ml-1 inline-flex items-center gap-1.5 rounded-md border border-border/60 px-2 py-1 text-[12px] text-foreground transition-colors hover:border-border hover:bg-fg/[0.05] disabled:opacity-40"
        >
          <Send className="size-3.5" />
          Send
        </button>
      </div>

      {/* canvas */}
      <div ref={wrapRef} className="relative min-h-0 flex-1" style={{ backgroundColor: bg }}>
        <canvas
          ref={canvasRef}
          className="absolute inset-0 touch-none"
          style={{ cursor: tool === 'eraser' ? 'cell' : 'crosshair' }}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
        />
        {empty && (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
            <p className="text-[12.5px] text-muted-foreground/70">
              Draw here, then send it to the agent.
            </p>
          </div>
        )}
      </div>

      <div className="shrink-0 border-t border-border/60 px-2.5 py-1 text-[11px] text-muted-foreground/70">
        {strokes.current.length} stroke{strokes.current.length === 1 ? '' : 's'} ·{' '}
        {baseWidth}px
      </div>
    </div>
  );
}

function ToolBtn({
  children,
  active,
  disabled,
  title,
  onClick,
}: {
  children: React.ReactNode;
  active?: boolean;
  disabled?: boolean;
  title: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      title={title}
      onClick={onClick}
      disabled={disabled}
      className={cn(
        'flex size-6 shrink-0 items-center justify-center rounded transition-colors disabled:opacity-30',
        active
          ? 'bg-fg/[0.1] text-foreground'
          : 'text-muted-foreground hover:bg-fg/[0.05] hover:text-foreground',
      )}
    >
      {children}
    </button>
  );
}
