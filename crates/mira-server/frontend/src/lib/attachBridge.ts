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

/** Queue a file for attachment to the next-mounted composer. */
export function attachFilesToComposer(files: File | File[]): void {
  for (const file of Array.isArray(files) ? files : [files]) {
    window.dispatchEvent(new CustomEvent<File>(ATTACH_FILE_EVENT, { detail: file }));
  }
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
