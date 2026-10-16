/**
 * Browser annotations: a note pinned to a point or an area of the agent
 * browser's page. Each one lands in the composer as a card (see
 * `BrowserNoteCard`) and is sent as a line of text plus a screenshot with
 * the mark drawn on it.
 *
 * Nothing is sent from here: the note waits in the composer like a quote,
 * so the user can add to it, annotate more, or drop it.
 */
import { composeNote, dataUrlToFile } from './attachBridge';

/** A mark in the page's CSS pixels: a point, or an area when `w`/`h` are set. */
export type Mark = { x: number; y: number; w?: number; h?: number };

export const isArea = (m: Mark) => (m.w ?? 0) > 0 && (m.h ?? 0) > 0;

/** The mark's anchor for describing what's under it: an area's centre. */
function anchor(m: Mark) {
  return isArea(m) ? { x: m.x + (m.w ?? 0) / 2, y: m.y + (m.h ?? 0) / 2 } : { x: m.x, y: m.y };
}

const ROLE_NAMES: Record<string, string> = {
  a: 'link', button: 'button', input: 'field', textarea: 'text box', select: 'menu', img: 'image',
  h1: 'heading', h2: 'heading', h3: 'heading', h4: 'heading', h5: 'heading', h6: 'heading', label: 'label',
};

/** What's under the mark, in a few words (`the "Sign in" button`), read from
 *  the live page. Null when the page can't be asked (no browser, a dialog). */
export async function describeAt(m: Mark): Promise<string | null> {
  const { x, y } = anchor(m);
  const expression =
    `(function(x,y){const el=document.elementFromPoint(x,y);if(!el)return null;` +
    `const t=(el.getAttribute('aria-label')||el.innerText||el.value||el.alt||el.title||'').trim().replace(/\\s+/g,' ');` +
    `return JSON.stringify({tag:el.tagName.toLowerCase(),role:el.getAttribute('role')||'',text:t.slice(0,80)});})(${x},${y})`;
  try {
    const res = await fetch('/api/browser/action', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ action: { action: 'evaluate', expression } }),
    });
    if (!res.ok) return null;
    const { text } = (await res.json()) as { text?: string };
    const el = JSON.parse(text ?? 'null') as { tag: string; role: string; text: string } | null;
    if (!el) return null;
    const kind = el.role || ROLE_NAMES[el.tag] || el.tag;
    // A container's text is everything inside it: name the thing, not that.
    const label = el.text.length > 48 ? `${el.text.slice(0, 40).trimEnd()}…` : el.text;
    return label ? `the "${label}" ${kind}` : `a ${kind}`;
  } catch {
    return null;
  }
}

const BLUE = '#2f6bff';

/** The pin: a blue drop whose point sits on the marked spot. */
function drawPin(ctx: CanvasRenderingContext2D, x: number, y: number, k: number) {
  const s = 18 * k;
  ctx.save();
  ctx.shadowColor = 'rgba(0,0,0,0.3)';
  ctx.shadowBlur = 6 * k;
  ctx.shadowOffsetY = 2 * k;
  ctx.fillStyle = BLUE;
  ctx.beginPath();
  ctx.roundRect(x, y, s, s, [2 * k, s / 2, s / 2, s / 2]);
  ctx.fill();
  ctx.restore();
}

/** Draw the mark over the frame, at the image's own pixel density. */
function drawMark(ctx: CanvasRenderingContext2D, m: Mark, k: number) {
  if (isArea(m)) {
    ctx.lineWidth = 2 * k;
    ctx.strokeStyle = BLUE;
    ctx.strokeRect(m.x * k, m.y * k, (m.w ?? 0) * k, (m.h ?? 0) * k);
  }
  drawPin(ctx, m.x * k, m.y * k, k);
}

async function loadFrame(src: string) {
  const img = new Image();
  img.src = src;
  await img.decode();
  return img;
}

/** The current frame with the mark drawn on it, as a PNG file. `frame` is
 *  the page size in CSS pixels; the image itself is at device pixels. */
async function renderMarked(img: HTMLImageElement, frame: { w: number }, m: Mark): Promise<File> {
  const canvas = document.createElement('canvas');
  canvas.width = img.naturalWidth;
  canvas.height = img.naturalHeight;
  const ctx = canvas.getContext('2d')!;
  ctx.drawImage(img, 0, 0);
  drawMark(ctx, m, img.naturalWidth / frame.w);
  return dataUrlToFile(canvas.toDataURL('image/png'), 'browser-note.png');
}

/** A small crop around the mark, for the composer card. */
function renderThumb(img: HTMLImageElement, frame: { w: number; h: number }, m: Mark): string {
  const k = img.naturalWidth / frame.w;
  // Frame the mark with some page around it, at the card's 3:2 shape.
  const pad = 36;
  let w = Math.max((m.w ?? 0) + pad * 2, 180);
  let h = Math.max((m.h ?? 0) + pad * 2, 120);
  if (w / h > 1.5) h = w / 1.5;
  else w = h * 1.5;
  const cx = m.x + (m.w ?? 0) / 2;
  const cy = m.y + (m.h ?? 0) / 2;
  const x = Math.min(Math.max(cx - w / 2, 0), Math.max(frame.w - w, 0));
  const y = Math.min(Math.max(cy - h / 2, 0), Math.max(frame.h - h, 0));
  const out = document.createElement('canvas');
  out.width = 192;
  out.height = 128;
  const ctx = out.getContext('2d')!;
  ctx.fillStyle = '#fff';
  ctx.fillRect(0, 0, out.width, out.height);
  const scale = out.width / (w * k);
  ctx.scale(scale, scale);
  ctx.translate(-x * k, -y * k);
  ctx.drawImage(img, 0, 0);
  drawMark(ctx, m, k);
  return out.toDataURL('image/png');
}

/** Hand a note to the composer, where it waits as a card. */
export async function annotationToComposer(opts: {
  owner?: string;
  src: string;
  frame: { w: number; h: number };
  url: string;
  mark: Mark;
  comment: string;
}): Promise<void> {
  const { src, frame, url, mark, comment } = opts;
  const [img, target] = await Promise.all([loadFrame(src), describeAt(mark)]);
  const image = await renderMarked(img, frame, mark);
  composeNote({ owner: opts.owner, url, target, comment: comment.trim(), image, thumb: renderThumb(img, frame, mark) });
}
