import { cn } from '@/lib/utils';
import { File as FileIcon, X } from 'lucide-react';
import { basename } from './ComposerMenus';
import { Attachment, ImageData } from './types';
/** Longest edge sent to the model. Bigger screenshots are scaled down —
 *  providers downscale anyway, and it keeps the payload small. */
export const IMAGE_MAX_EDGE = 1568;

/** Read an image file, scaled down to IMAGE_MAX_EDGE, as base64. */
export async function readImage(file: File): Promise<ImageData> {
  let image: ImageBitmap | HTMLImageElement;
  let objectUrl: string | undefined;
  try {
    image = await createImageBitmap(file);
  } catch {
    // Older desktop webviews cannot decode every format through ImageBitmap.
    objectUrl = URL.createObjectURL(file);
    const element = new Image();
    image = await new Promise<HTMLImageElement>((resolve, reject) => {
      element.onload = () => resolve(element);
      element.onerror = () => {
        URL.revokeObjectURL(objectUrl!);
        reject(new Error('This image could not be decoded.'));
      };
      element.src = objectUrl!;
    });
  }
  try {
    const width = image instanceof HTMLImageElement ? image.naturalWidth : image.width;
    const height = image instanceof HTMLImageElement ? image.naturalHeight : image.height;
    const scale = Math.min(1, IMAGE_MAX_EDGE / Math.max(width, height));
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(width * scale));
    canvas.height = Math.max(1, Math.round(height * scale));
    const context = canvas.getContext('2d');
    if (!context) throw new Error('Image processing is unavailable.');
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
    const media_type = file.type === 'image/jpeg' ? 'image/jpeg' : 'image/png';
    const url = canvas.toDataURL(media_type, 0.9);
    return {
      media_type,
      data: url.slice(url.indexOf(',') + 1),
      source: {
        name: file.name,
        width: canvas.width,
        height: canvas.height,
        ...(file as File & { source?: import('../../types').ImageSource }).source,
      },
    };
  } finally {
    if (objectUrl) URL.revokeObjectURL(objectUrl);
    if ('close' in image) image.close();
  }
}

/** Cap on how many bytes we'll inline from a single OS-picked file. Larger
 *  files still get a chip in the composer, but the inlined body is
 *  truncated with a marker so a rogue 50 MB video doesn't nuke the model's
 *  context window. Text files usually clock in well under this. */
export const NATIVE_ATTACH_MAX_BYTES = 256 * 1024;

/* ---------- attachment chip ---------- */

export function AttachmentChip({
  attachment,
  cwd,
  onRemove,
}: {
  attachment: Attachment;
  cwd: string;
  onRemove: () => void;
}) {
  const label = relativeTo(attachment.path, cwd);
  // "Area.mp4" → filename "Area.mp4", subtype badge "MP4". Files without
  // an extension (`Makefile`) fall back to the byte size as the subtype
  // so the two-line chip still fills sensibly.
  const dot = attachment.path.lastIndexOf('.');
  const filename = label;
  const subtype =
    dot > 0 && dot < attachment.path.length - 1
      ? attachment.path.slice(dot + 1).toUpperCase()
      : formatBytes(attachment.bytes);
  return (
    <span
      className={cn(
        // Codex-style chip: rounded card with a padded file-icon square on the
        // left, filename bold above a muted subtype (extension). Overflow-wide
        // filenames truncate — full path lives in the tooltip.
        'group relative inline-flex items-center gap-2.5 rounded-xl border border-border/60 bg-background/70 py-1.5 pl-2 pr-8',
      )}
      title={`${attachment.path} · ${formatBytes(attachment.bytes)}`}
    >
      <span className="inline-flex size-8 shrink-0 items-center justify-center rounded-md border border-border/70 bg-secondary/70 text-muted-foreground">
        <FileIcon className="size-4" />
      </span>
      <span className="flex min-w-0 flex-col">
        <span className="max-w-[18rem] truncate text-[13px] font-semibold text-foreground">
          {filename}
        </span>
        <span className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground/80">
          {subtype}
        </span>
      </span>
      <button
        type="button"
        onClick={onRemove}
        className="absolute right-1.5 top-1.5 inline-flex size-4 items-center justify-center rounded-full bg-secondary/90 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        aria-label={`Remove ${label}`}
      >
        <X className="size-2.5" />
      </button>
    </span>
  );
}

export function relativeTo(abs: string, base: string): string {
  if (base && abs.startsWith(base + '/')) return abs.slice(base.length + 1);
  if (base && abs === base) return '.';
  const parts = abs.split('/');
  if (parts.length <= 3) return abs;
  return '…/' + parts.slice(-2).join('/');
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/** Inline attachments at the top of the message the model receives. Uses
 *  a stable "## Attached files" header and per-file fenced blocks with the
 *  path as the info string, so the model can trivially cite `path:line` in
 *  its reply. Language guess = extension. */
export function renderAttachments(atts: Attachment[], cwd: string): string {
  const parts = atts.map((a) => {
    const rel = relativeTo(a.path, cwd);
    const lang = extToLang(a.path);
    return `### ${rel}\n\`\`\`${lang}\n${a.content}\n\`\`\``;
  });
  return `## Attached files\n\n${parts.join('\n\n')}`;
}

// Parse the "## Attached files" header out of a user message body so the
// transcript can render each file as a Codex-style chip above the bubble
// instead of dumping the raw fenced content into the reader's face. If
// the body doesn't start with the marker, returns the body unchanged and
// an empty `attachments` list.
//
// Hand-walks the string rather than regexing — the header shape is very
// regular and a manual walk sidesteps the edge cases (backtick fencing +
// non-greedy quantifiers) that made an earlier regex-based parser
// mis-slice filenames on multi-attachment messages.
export function parseSentAttachments(body: string): {
  attachments: Array<{ filename: string; subtype: string }>;
  text: string;
} {
  const marker = '## Attached files';
  if (!body.startsWith(marker)) {
    return { attachments: [], text: body };
  }

  // Skip the marker + up to two trailing newlines. Empty-line-before-the-
  // first-header is what `renderAttachments` writes, but be lenient.
  let i = marker.length;
  while (i < body.length && body[i] === '\n') i++;

  const attachments: Array<{ filename: string; subtype: string }> = [];
  while (i < body.length) {
    // Each block must start with "### " (space required). Anything else
    // is the user's own text — bail so it survives into `text` below.
    if (!body.startsWith('### ', i)) break;
    i += 4;

    // Filename runs to the next newline.
    const nl = body.indexOf('\n', i);
    if (nl < 0) break;
    const filename = basename(body.slice(i, nl).trim());
    i = nl + 1;

    // Opening fence line: "```<lang>\n". Fence is exactly three
    // backticks at the start of the line; lang may be empty.
    if (!body.startsWith('```', i)) break;
    const fenceNl = body.indexOf('\n', i);
    if (fenceNl < 0) break;
    i = fenceNl + 1;

    // Content runs until a line that is exactly "```". Scan
    // line-by-line so a `\`\`\`` embedded mid-line (unlikely for us, but
    // possible in inlined source code) can't fake a fence.
    let closed = false;
    while (i < body.length) {
      const eol = body.indexOf('\n', i);
      const line = eol < 0 ? body.slice(i) : body.slice(i, eol);
      const advance = eol < 0 ? body.length : eol + 1;
      if (line === '```') {
        i = advance;
        closed = true;
        break;
      }
      i = advance;
    }
    if (!closed) break;

    // Blank lines between blocks (or before the user text) — swallow.
    while (i < body.length && body[i] === '\n') i++;

    const dot = filename.lastIndexOf('.');
    const subtype =
      dot > 0 && dot < filename.length - 1 ? filename.slice(dot + 1).toUpperCase() : 'FILE';
    attachments.push({ filename, subtype });
  }

  const text = body.slice(i);
  return { attachments, text };
}

/** Read-only rendering of an attachment chip for the transcript. Same
 *  Codex-style visual as the composer chip, minus the X (nothing to
 *  remove on a sent message). */
export function SentAttachmentChip({ filename, subtype }: { filename: string; subtype: string }) {
  return (
    <span
      className="inline-flex items-center gap-2.5 rounded-xl border border-border/60 bg-background/70 py-1.5 pl-2 pr-3"
      title={filename}
    >
      <span className="inline-flex size-8 shrink-0 items-center justify-center rounded-md border border-border/70 bg-secondary/70 text-muted-foreground">
        <FileIcon className="size-4" />
      </span>
      <span className="flex min-w-0 flex-col">
        <span className="max-w-[18rem] truncate text-[13px] font-semibold text-foreground">
          {filename}
        </span>
        <span className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground/80">
          {subtype}
        </span>
      </span>
    </span>
  );
}

/** Cheap "should we skip reading this as text?" heuristic. Trusts the
 *  browser-supplied MIME type first (image/*, video/*, audio/*, and the
 *  usual binary application/* families), then falls back to an extension
 *  denylist so files with no MIME (Finder-attached mp4, exe, zip) still
 *  land as binary. Anything unrecognised is treated as text — the
 *  inlined content is capped separately so a mis-guess still can't
 *  blow out the message. */
export function looksBinary(file: File): boolean {
  const t = (file.type || '').toLowerCase();
  if (t.startsWith('image/') || t.startsWith('video/') || t.startsWith('audio/')) return true;
  if (
    t === 'application/pdf' ||
    t === 'application/zip' ||
    t === 'application/x-tar' ||
    t === 'application/x-gzip' ||
    t === 'application/octet-stream'
  )
    return true;
  const name = file.name.toLowerCase();
  const dot = name.lastIndexOf('.');
  if (dot < 0) return false;
  const ext = name.slice(dot + 1);
  const binaryExts = new Set([
    'png',
    'jpg',
    'jpeg',
    'gif',
    'webp',
    'bmp',
    'ico',
    'heic',
    'heif',
    'svg',
    'mp4',
    'mov',
    'mkv',
    'webm',
    'avi',
    'm4v',
    'mp3',
    'wav',
    'flac',
    'aac',
    'ogg',
    'm4a',
    'pdf',
    'zip',
    'gz',
    'tar',
    'tgz',
    'bz2',
    '7z',
    'rar',
    'exe',
    'dll',
    'so',
    'dylib',
    'bin',
    'wasm',
    'ttf',
    'otf',
    'woff',
    'woff2',
    'psd',
    'sketch',
    'fig',
  ]);
  return binaryExts.has(ext);
}

export function extToLang(path: string): string {
  const dot = path.lastIndexOf('.');
  if (dot < 0) return '';
  const ext = path.slice(dot + 1).toLowerCase();
  const map: Record<string, string> = {
    ts: 'ts',
    tsx: 'tsx',
    js: 'js',
    jsx: 'jsx',
    py: 'python',
    rb: 'ruby',
    go: 'go',
    rs: 'rust',
    java: 'java',
    kt: 'kotlin',
    swift: 'swift',
    c: 'c',
    h: 'c',
    cpp: 'cpp',
    hpp: 'cpp',
    cs: 'csharp',
    php: 'php',
    sh: 'bash',
    bash: 'bash',
    zsh: 'bash',
    yml: 'yaml',
    yaml: 'yaml',
    toml: 'toml',
    json: 'json',
    md: 'md',
    html: 'html',
    css: 'css',
    scss: 'scss',
    sql: 'sql',
  };
  return map[ext] ?? '';
}
