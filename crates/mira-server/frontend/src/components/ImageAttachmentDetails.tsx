import { useTranscriptDisclosure } from './TranscriptDisclosure';
import type { ImageAttachment } from '../types';
export function ImageAttachmentDetails({ image }: { image: ImageAttachment }) {
  const source = image.source;
  const [open, setOpen] = useTranscriptDisclosure(`image-details:${source?.name}:${source?.captured_at ?? image.data.slice(-64)}`);
  if (!source) return null;
  return <details open={open} onToggle={event => setOpen(event.currentTarget.open)} className="max-w-[240px] text-left text-[11px] text-muted-foreground">
    <summary className="cursor-pointer truncate" title={source.window ?? source.name}>{source.window ?? source.name}</summary>
    <div className="mt-1 space-y-1"><p>{source.name}{source.width && source.height ? ` · ${source.width} × ${source.height}` : ''}</p>{source.captured_at && <p>{new Date(source.captured_at).toLocaleString()}</p>}{source.accessible_text && <details><summary className="cursor-pointer">Captured text</summary><pre className="max-h-40 overflow-auto whitespace-pre-wrap">{source.accessible_text}</pre></details>}</div>
  </details>;
}
