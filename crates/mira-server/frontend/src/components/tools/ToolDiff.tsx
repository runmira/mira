import { FileText } from 'lucide-react';
import { useMemo } from 'react';
import type { DiffLine, DiffPreview, ToolCall } from '../../types';
import { InlineDiff } from '../diffs/LazyDiffs';
import { Tip } from '../ui/Tip';
import { safeParse } from './arguments';
import { CommandBlock, pickProseText } from './ToolResult';
import { ARG_PREVIEW_REDUNDANT } from './types';
/**
 * Codex-style diff card. Elevated background, its own header row
 * showing the file icon + short filename + full path (muted) + the
 * `+N -N` summary, and line numbers on the left of every diff row.
 *
 * Line numbers are sequential (1..N) rather than source-file
 * positions — the raw `DiffPreview` doesn't carry hunk metadata, and
 * a running counter matches the Codex look while giving the reader
 * a visual guide. When a full source line-number scheme lands
 * upstream (hunk headers or LSP positions), this component swaps in
 * the real numbers.
 */
export function DiffCard({
  filePath,
  tool,
  lines,
  stats,
  diffPreview,
  onOpenFile,
}: {
  filePath: string;
  tool: string;
  lines: DiffLine[];
  stats: { adds: number; dels: number } | null;
  diffPreview: DiffPreview | null;
  onOpenFile?: (path: string, diff: DiffPreview | null) => void;
}) {
  const shortName = filePath ? (filePath.split('/').pop() ?? filePath) : tool;
  const dir =
    filePath && filePath.includes('/') ? filePath.slice(0, filePath.lastIndexOf('/')) : '';

  return (
    <div className="overflow-hidden rounded-lg border border-border/50 bg-mira-elev1/60">
      <div className="flex items-center gap-2 border-b border-border/40 bg-mira-elev1/80 px-3 py-2 text-[12px]">
        <FileText className="size-3.5 shrink-0 text-muted-foreground" />
        {onOpenFile && filePath ? (
          <Tip label="Open in file viewer" className="shrink-0">
            <button
              type="button"
              onClick={() => onOpenFile(filePath, diffPreview)}
              className="font-medium text-foreground transition-colors hover:text-mira-blue hover:underline underline-offset-2"
            >
              {shortName}
            </button>
          </Tip>
        ) : (
          <span className="shrink-0 font-medium text-foreground">{shortName}</span>
        )}
        {dir && <span className="min-w-0 flex-1 truncate text-muted-foreground/70">{dir}</span>}
        {stats && (
          <span className="ml-auto shrink-0 font-mono text-[11.5px]">
            <span className="text-emerald-400">+{stats.adds}</span>{' '}
            <span className="text-rose-400">-{stats.dels}</span>
          </span>
        )}
      </div>
      <InlineDiff path={filePath || tool} lines={lines} />
    </div>
  );
}

/** Count adds vs deletions in a DiffLine list — powers the `+N -N`
 *  chip in both the compact summary row and the diff card header. */
export function countDiff(lines: DiffLine[]): { adds: number; dels: number } {
  let adds = 0;
  let dels = 0;
  for (const l of lines) {
    if (l.tag === 'add') adds += 1;
    else if (l.tag === 'del') dels += 1;
  }
  return { adds, dels };
}

/**
 * Fallback body shown when a live `DiffPreview` isn't available — most
 * commonly after a session reload, since the server currently doesn't
 * persist previews. Renders each known tool in a shape that reads well:
 *
 *   edit_file  → pseudo-diff (- old / + new)
 *   write_file → all-adds pseudo-diff
 *   bash       → shell-style block with a `$` prompt and real newlines
 *   grep       → `pattern` on one line + path hint
 *   read_file  → the file path
 *   rustfmt    → the file path
 *
 * Anything else falls through to the pretty-printed JSON — that's still
 * the least-bad rendering for unknown args shapes.
 */
export function ReconstructedPreview({ call }: { call: ToolCall }) {
  const args = useMemo(() => safeParse(call.function.arguments), [call.function.arguments]);
  const tool = call.function.name;
  const pseudo = useMemo(() => reconstructDiff(tool, args), [tool, args]);

  if (pseudo) {
    const path =
      typeof args?.path === 'string'
        ? args.path
        : typeof args?.file_path === 'string'
          ? args.file_path
          : tool;
    return (
      <InlineDiff
        path={path}
        lines={pseudo}
        maxHeight="20rem"
        className="rounded-lg border border-border/50"
      />
    );
  }

  // Bash: render the command as a proper shell block. JSON-escaped `\n`s
  // in the raw args become real line breaks; multi-line heredocs, chained
  // `&&`s, etc. all read like they would in a terminal.
  if (tool === 'bash' && typeof args?.command === 'string') {
    return <CommandBlock command={String(args.command)} />;
  }

  // Skip arg echo entirely when the row header already carries the
  // meaningful arg (glob pattern, grep pattern, read path, etc.).
  // The row above and the result below say everything we need; a
  // duplicated arg block just adds visual weight.
  if (tool in ARG_PREVIEW_REDUNDANT) {
    return null;
  }

  // Text-heavy single-field tools — memory writes, subagent prompts,
  // ask-user proposals, task subjects — get rendered as prose in a
  // quiet elevated block so a paragraph reads as a paragraph, not
  // as a JSON payload.
  const proseText = pickProseText(tool, args);
  if (proseText) {
    return (
      <div className="rounded-md border border-border/40 bg-mira-elev1/50 px-3 py-2 text-[12.5px] leading-relaxed text-foreground/85">
        <div className="whitespace-pre-wrap break-words">{proseText}</div>
      </div>
    );
  }

  // Nothing worth showing — args are either empty or too weird to
  // render nicely. Drop the block entirely rather than fall back to
  // raw JSON (which was the old failure mode).
  return null;
}

/** Best-effort before/after reconstruction from raw tool args. Returns
 *  `null` for tools where a diff view doesn't apply. */
export function reconstructDiff(tool: string, args: any): DiffLine[] | null {
  if (!args) return null;
  if (
    tool === 'edit_file' &&
    typeof args.old_string === 'string' &&
    typeof args.new_string === 'string'
  ) {
    const oldLines = String(args.old_string).split('\n');
    const newLines = String(args.new_string).split('\n');
    const lines: DiffLine[] = [];
    for (const t of oldLines) lines.push({ tag: 'del', text: t });
    for (const t of newLines) lines.push({ tag: 'add', text: t });
    return lines;
  }
  if (tool === 'write_file' && typeof args.content === 'string') {
    const contentLines = String(args.content).split('\n');
    // No old side (fresh write) — render as all-adds.
    return contentLines.map<DiffLine>((text) => ({ tag: 'add', text }));
  }
  return null;
}

export function DiffRow({ line }: { line: DiffLine }) {
  if (line.tag === 'hunkgap') return <div className="diff-line hunk">···</div>;
  const cls = line.tag === 'add' ? 'add' : line.tag === 'del' ? 'del' : 'ctx';
  const prefix = line.tag === 'add' ? '+' : line.tag === 'del' ? '-' : ' ';
  return (
    <div className={`diff-line ${cls}`}>
      <span className="prefix">{prefix}</span>
      <span className="text">{line.text}</span>
    </div>
  );
}
