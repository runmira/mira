/**
 * A tool's change, drawn inline in the chat with the same renderer as the
 * review drawer and file viewer: syntax colors, in-line highlights, the
 * app theme and Settings → Appearance → Diffs.
 *
 * Unlike those panels it isn't virtualized (FileDiff, not CodeView), so it
 * takes its natural height and scrolls past `maxHeight`.
 */
import { useMemo } from 'react';
import { FileDiff } from '@pierre/diffs/react';
import { parseDiffFromFile } from '@pierre/diffs';
import type { DiffLine } from '../../types';
import { DIFF_SURFACE_THEME_UNSAFE_CSS } from '@/lib/diffRender';
import { useDiffOptions } from '@/lib/diffPrefs';
import { DiffWorkerPoolProvider } from './DiffWorkerPoolProvider';
import { cn } from '@/lib/utils';

/** Past this, the inline preview stops; the file viewer has the rest. */
const MAX_LINES = 400;

/** The card draws its own header, so drop the gap the renderer leaves
 *  under the (hidden) file header. */
const INLINE_UNSAFE_CSS = `${DIFF_SURFACE_THEME_UNSAFE_CSS}
pre[data-diff] > [data-code] { padding-top: 0 !important; }`;

export function InlineDiff({
  path,
  lines,
  maxHeight = '50vh',
  className,
}: {
  path: string;
  lines: DiffLine[];
  maxHeight?: string;
  className?: string;
}) {
  const options = useDiffOptions();
  const capped = lines.length > MAX_LINES ? lines.slice(0, MAX_LINES) : lines;
  // Rebuild the two sides from the preview lines and let the renderer diff
  // them (as the file viewer does). Excerpt gaps are joined: line numbers
  // are the excerpt's own, as before.
  const fileDiff = useMemo(() => {
    const side = (keep: 'add' | 'del') =>
      capped
        .map((l) => (l.tag === 'ctx' || l.tag === keep ? l.text : null))
        .filter((t): t is string => t !== null)
        // A trailing newline: the preview's lines are whole lines, so the
        // last one isn't missing one (else every card says "No newline at
        // end of file").
        .join('\n') + '\n';
    try {
      return parseDiffFromFile(
        { name: path || 'file', contents: side('del') },
        { name: path || 'file', contents: side('add') },
        { context: Infinity },
      );
    } catch {
      return null;
    }
  }, [path, capped]);

  if (!fileDiff) return null;
  return (
    <div className={cn('overflow-auto', className)} style={{ maxHeight }}>
      <DiffWorkerPoolProvider>
        <FileDiff
          fileDiff={fileDiff}
          className="diff-render-surface"
          options={{ ...options, disableFileHeader: true, unsafeCSS: INLINE_UNSAFE_CSS }}
        />
      </DiffWorkerPoolProvider>
      {lines.length > MAX_LINES && (
        <div className="border-t border-border/30 px-3 py-1.5 text-center text-[11px] text-muted-foreground/70">
          … {lines.length - MAX_LINES} more lines — open the file to see them all
        </div>
      )}
    </div>
  );
}
