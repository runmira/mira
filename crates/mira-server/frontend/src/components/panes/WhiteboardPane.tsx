import { useCallback, useEffect, useRef, useState } from 'react';
import { ChevronDown, Eraser, Pencil, Redo2, Send, Trash2, Undo2 } from 'lucide-react';
import { useTheme, type Theme } from '@/lib/theme';
import { cn } from '@/lib/utils';

/** One continuous pen-down → pen-up gesture. */
type Stroke = {
  /** A palette id (resolved per theme when drawn, so a drawing recolors
   *  with the theme), or `erase`. */
  color: InkId | 'erase';
  width: number;
  /** Flat [x0,y0,x1,y1,…] in CSS pixels relative to the canvas. */
  points: number[];
};

type Tool = 'pen' | 'eraser';

/** Inks as [dark-theme, light-theme] shades: the pastel that reads on a
 *  dark board turns to mush on a light one, and the default ink has to
 *  flip between near-white and near-black. */
const PALETTE = [
  { id: 'ink', name: 'Ink', dark: '#e4e4e7', light: '#1f2328' },
  { id: 'red', name: 'Red', dark: '#f7768e', light: '#d1242f' },
  { id: 'amber', name: 'Amber', dark: '#ffb86b', light: '#bc4c00' },
  { id: 'cyan', name: 'Cyan', dark: '#7dcfff', light: '#0a7ea4' },
  { id: 'green', name: 'Green', dark: '#a6d189', light: '#1a7f37' },
  { id: 'blue', name: 'Blue', dark: '#7aa2f7', light: '#3b5bdb' },
  { id: 'purple', name: 'Purple', dark: '#bb9af7', light: '#8250df' },
] as const;
type InkId = (typeof PALETTE)[number]['id'];

function inkColor(id: InkId, theme: Theme): string {
  const p = PALETTE.find((c) => c.id === id) ?? PALETTE[0];
  return theme === 'light' ? p.light : p.dark;
}

/** The board itself, per theme (matches the terminal's surfaces). */
const BOARD = { dark: '#1a1c20', light: '#fbfbfc' } as const;

/** Below this toolbar width, colors and sizes fold into one popover so
 *  the bar stays a single row. */
const COMPACT_BELOW = 470;

const WIDTHS = [2, 4, 8];

/** Index-addressed base stroke widths — the width buttons map 1:1. */
const PALETTE_BASE = WIDTHS;

/** Eraser width multiplier. Erasing is a stroke too (drawn with
 *  `destination-out`), and every repaint replays the stroke list from
 *  scratch, so undo brings erased ink back. */
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
  const [color, setColor] = useState<InkId>('ink');
  const [widthIdx, setWidthIdx] = useState(1);
  /** Bumped to force a redraw of the committed strokes after undo/clear. */
  const [version, setVersion] = useState(0);
  const [empty, setEmpty] = useState(true);

  const barRef = useRef<HTMLDivElement | null>(null);
  const [compact, setCompact] = useState(false);
  const [inksOpen, setInksOpen] = useState(false);
  useEffect(() => {
    const el = barRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setCompact(el.clientWidth < COMPACT_BELOW));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const strokes = useRef<Stroke[]>([]);
  const redoStack = useRef<Stroke[]>([]);
  const live = useRef<Stroke | null>(null);
  const drawing = useRef(false);

  const theme = useTheme();
  const themeRef = useRef(theme);
  themeRef.current = theme;
  const bg = BOARD[theme];

  /** Redraw everything: committed strokes, then the in-progress one. */
  const repaint = useCallback(() => {
    const cv = canvasRef.current;
    if (!cv) return;
    const ctx = cv.getContext('2d');
    if (!ctx) return;

    // Points are stored in CSS pixels; the backing store is CSS size × DPR.
    // Scale once here so a point lands under the pointer (drawing raw CSS
    // coordinates into a 2× store put every stroke up and to the left).
    const dpr = window.devicePixelRatio || 1;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, cv.width, cv.height);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    const draw = (s: Stroke) => {
      const pts = s.points;
      if (pts.length < 2) return;
      ctx.save();
      ctx.lineCap = 'round';
      ctx.lineJoin = 'round';
      ctx.lineWidth = s.width;
      if (s.color === 'erase') {
        ctx.globalCompositeOperation = 'destination-out';
        ctx.strokeStyle = '#000';
      } else {
        ctx.strokeStyle = inkColor(s.color, themeRef.current);
      }
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

    for (const s of strokes.current) draw(s);
    if (live.current) draw(live.current);
  }, []);

  /** Match the backing store to the element's CSS size × DPR. Returns true
   *  when the size changed, which is the signal to repaint. */
  const syncSize = useCallback((): boolean => {
    const cv = canvasRef.current;
    const wrap = wrapRef.current;
    if (!cv || !wrap) return false;
    const dpr = window.devicePixelRatio || 1;
    // Layout size, not getBoundingClientRect: under the interface-size
    // zoom the rect is in zoomed pixels while the canvas lays out in CSS
    // pixels.
    const w = Math.max(1, Math.floor(wrap.clientWidth));
    const h = Math.max(1, Math.floor(wrap.clientHeight));
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

  // Committed strokes only change through undo/redo/clear — and their
  // colors change with the theme.
  useEffect(() => {
    repaint();
  }, [version, theme, repaint]);

  function pointFromEvent(e: React.PointerEvent<HTMLCanvasElement>): [number, number] {
    const cv = canvasRef.current!;
    const rect = cv.getBoundingClientRect();
    // The rect (and clientX/Y) are in viewport pixels, which differ from
    // the canvas's CSS pixels when the page is zoomed (Settings → Interface
    // size). Map through the ratio so the stroke stays under the pointer.
    const sx = rect.width ? cv.clientWidth / rect.width : 1;
    const sy = rect.height ? cv.clientHeight / rect.height : 1;
    return [(e.clientX - rect.left) * sx, (e.clientY - rect.top) * sy];
  }

  function onPointerDown(e: React.PointerEvent<HTMLCanvasElement>) {
    if (e.button !== 0 && e.pointerType === 'mouse') return;
    e.currentTarget.setPointerCapture(e.pointerId);
    drawing.current = true;
    redoStack.current = [];
    const [x, y] = pointFromEvent(e);
    live.current = {
      color: tool === 'eraser' ? 'erase' : color,
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

  const swatches = PALETTE.map((c) => (
    <button
      key={c.id}
      type="button"
      title={c.name}
      onClick={() => {
        setColor(c.id);
        setTool('pen');
        setInksOpen(false);
      }}
      className={cn(
        'size-4 shrink-0 rounded-full ring-1 ring-fg/15 transition-transform',
        color === c.id && tool === 'pen'
          ? 'ring-2 ring-foreground/70 ring-offset-1 ring-offset-background'
          : 'hover:scale-110',
      )}
      style={{ backgroundColor: theme === 'light' ? c.light : c.dark }}
    >
      <span className="sr-only">{c.name}</span>
    </button>
  ));

  const widths = WIDTHS.map((w, i) => (
    <button
      key={w}
      type="button"
      title={`${w}px`}
      onClick={() => setWidthIdx(i)}
      className={cn(
        'flex size-5 shrink-0 items-center justify-center rounded transition-colors',
        widthIdx === i ? 'bg-fg/[0.1]' : 'hover:bg-fg/[0.05]',
      )}
    >
      <span className="rounded-full bg-foreground/80" style={{ width: w + 2, height: w + 2 }} />
    </button>
  ));

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* toolbar — always one row; narrow panels fold the inks into a popover */}
      <div
        ref={barRef}
        className="flex shrink-0 items-center gap-1.5 border-b border-border/60 px-2.5 py-1.5"
      >
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

        <span className="mx-0.5 h-4 w-px shrink-0 bg-border" />

        {compact ? (
          <div className="relative">
            <button
              type="button"
              title="Color and size"
              aria-haspopup="dialog"
              aria-expanded={inksOpen}
              onClick={() => setInksOpen((v) => !v)}
              className={cn(
                'flex h-6 items-center gap-1.5 rounded-md px-1.5 transition-colors',
                inksOpen ? 'bg-fg/[0.1]' : 'hover:bg-fg/[0.05]',
              )}
            >
              <span
                className="size-3.5 rounded-full ring-1 ring-fg/20"
                style={{ backgroundColor: inkColor(color, theme) }}
              />
              <span
                className="rounded-full bg-foreground/70"
                style={{ width: baseWidth + 2, height: baseWidth + 2 }}
              />
              <ChevronDown className="size-3 text-muted-foreground" />
            </button>
            {inksOpen && (
              <>
                <div className="fixed inset-0 z-30" onClick={() => setInksOpen(false)} />
                <div className="absolute left-0 top-[calc(100%+6px)] z-40 flex w-max flex-col gap-2.5 rounded-lg border border-border bg-popover p-2.5 shadow-lg shadow-shade/20">
                  <div className="grid grid-cols-7 gap-1.5">{swatches}</div>
                  <div className="flex items-center gap-1 border-t border-border/60 pt-2">{widths}</div>
                </div>
              </>
            )}
          </div>
        ) : (
          <>
            <div className="flex shrink-0 items-center gap-1.5">{swatches}</div>
            <span className="mx-0.5 h-4 w-px shrink-0 bg-border" />
            <div className="flex shrink-0 items-center gap-0.5">{widths}</div>
          </>
        )}

        <span className="min-w-0 flex-1" />

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
          title="Send to the chat"
          className="ml-0.5 inline-flex shrink-0 items-center gap-1.5 rounded-md border border-border/60 px-2 py-1 text-[12px] text-foreground transition-colors hover:border-border hover:bg-fg/[0.05] disabled:opacity-40"
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
