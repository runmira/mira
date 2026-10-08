/**
 * Bridge for handing a `File` to the composer from anywhere in the app.
 *
 * The composer owns all the attachment intake — image downscaling, the
 * 256 KB inline cap, binary detection — and already accepts files from
 * drag-and-drop, paste, and the file picker. Anything that produces a file
 * out-of-band (the whiteboard pane's "Send") should reuse that path rather
 * than reimplement it, so it goes through here.
 *
 * Deliberately a `window` event: the composer is a ~2,800-line component
 * with no imperative handle, and adding one just to serve this would mean
 * restructuring its signature for a single caller.
 */

export const ATTACH_FILE_EVENT = 'mira:attach-file';

let intake: ((file: File) => void) | undefined;
const pending: File[] = [];

/** Deliver pending pane/drop attachments when the composer mounts. */
export function registerAttachmentIntake(listener: (file: File) => void): () => void {
  intake = listener;
  for (const file of pending.splice(0)) listener(file);
  return () => { if (intake === listener) intake = undefined; };
}

/** Queue a file for attachment to the next-mounted composer. */
export function attachFilesToComposer(files: File | File[]): void {
  for (const file of Array.isArray(files) ? files : [files]) {
    if (intake) intake(file);
    else pending.push(file);
  }
}

export const COMPOSE_TEXT_EVENT = 'mira:compose-text';

/** Put `text` in the composer (after whatever is already typed) and focus
 *  it — for panes that hand the user a ready prompt ("Fix this failure",
 *  "Send to agent") without sending it behind their back. */
export function composeText(text: string): void {
  window.dispatchEvent(new CustomEvent<string>(COMPOSE_TEXT_EVENT, { detail: text }));
}

export const COMPOSE_QUOTE_EVENT = 'mira:compose-quote';

/** A passage of a reply, quoted into the composer as a card. `href` is its
 *  citation link: clicking the card jumps back to the original. */
export type ComposerQuote = { id: string; text: string; turn: string; href: string };

export function composeQuote(quote: Omit<ComposerQuote, 'id'>): void {
  const id = `q-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
  window.dispatchEvent(new CustomEvent<ComposerQuote>(COMPOSE_QUOTE_EVENT, { detail: { id, ...quote } }));
}

/* The passage "Ask in side chat" hands the aside pane. A store rather than
   an event: the pane may not be mounted yet when it's set. */
let asidePassage: string | null = null;
const asideListeners = new Set<() => void>();
export function setAsidePassage(text: string | null): void {
  asidePassage = text;
  asideListeners.forEach((l) => l());
}
export function getAsidePassage(): string | null {
  return asidePassage;
}
export function subscribeAsidePassage(listener: () => void): () => void {
  asideListeners.add(listener);
  return () => asideListeners.delete(listener);
}

/** `data:image/png;base64,…` → `File`, so the whiteboard can hand its
 *  canvas export to `attachFilesToComposer`. */
export async function dataUrlToFile(
  dataUrl: string,
  filename: string,
): Promise<File> {
  const res = await fetch(dataUrl);
  const blob = await res.blob();
  return new File([blob], filename, { type: blob.type || 'image/png' });
}
