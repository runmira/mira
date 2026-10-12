import { useTranscriptDisclosure } from './TranscriptDisclosure';
import type { ImageAttachment } from '../types';

/** Where an image came from, and anything readable off it.
 *
 *  The composer and the transcript show the same facts in different
 *  surfaces — a hover card while composing, a disclosure once sent — so
 *  the markup lives here once and each surface only chooses how to reveal
 *  it. */
export function ImageAttachmentMeta({ image }: { image: ImageAttachment }) {
  const source = image.source;
  if (!source) return null;
  return (
    <div className="space-y-1">
      <p className="truncate" title={source.name}>
        {source.name}
        {source.width && source.height ? ` · ${source.width} × ${source.height}` : ''}
      </p>
      {source.captured_at && <p>{new Date(source.captured_at).toLocaleString()}</p>}
      {source.accessible_text && (
        <details>
          <summary className="cursor-pointer">Captured text</summary>
          <pre className="max-h-40 overflow-auto whitespace-pre-wrap">{source.accessible_text}</pre>
        </details>
      )}
    </div>
  );
}

/** Sent-image provenance, inline under the thumbnail in the transcript.
 *
 *  A collapsed `<details>`, not a hover card: the transcript is a record,
 *  so the provenance should stay reachable to a keyboard and to a reader
 *  scrolling back, without depending on discovering a hover target. */
export function ImageAttachmentDetails({ image }: { image: ImageAttachment }) {
  const source = image.source;
  const [open, setOpen] = useTranscriptDisclosure(
    `image-details:${source?.name}:${source?.captured_at ?? (image.url ?? image.data.slice(-64))}`,
  );
  if (!source) return null;
  return (
    <details
      open={open}
      onToggle={(event) => setOpen(event.currentTarget.open)}
      className="max-w-[240px] text-left text-[11px] text-muted-foreground"
    >
      <summary className="cursor-pointer truncate" title={source.window ?? source.name}>
        {source.window ?? source.name}
      </summary>
      <ImageAttachmentMeta image={image} />
    </details>
  );
}