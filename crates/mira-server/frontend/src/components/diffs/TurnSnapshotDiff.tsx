import { useMemo } from 'react';
import { FileDiff } from '@pierre/diffs/react';
import { parseDiffFromFile } from '@pierre/diffs';
import { DiffWorkerPoolProvider } from './DiffWorkerPoolProvider';
import { DIFF_SURFACE_THEME_UNSAFE_CSS } from '../../lib/diffRender';
import { useDiffOptions } from '../../lib/diffPrefs';
/** Frozen complete versions preserve original line numbers and deletions. */
export function TurnSnapshotDiff({ path, before, after }: { path: string; before: string; after: string }) {
  const options = useDiffOptions();
  const fileDiff = useMemo(() => parseDiffFromFile({ name: path, contents: before }, { name: path, contents: after }), [path, before, after]);
  if (before === after) return <p className="py-2 text-xs text-muted-foreground">No text changes in this turn.</p>;
  return <div className="max-h-[480px] overflow-auto"><DiffWorkerPoolProvider><FileDiff fileDiff={fileDiff} className="diff-render-surface" options={{ ...options, disableFileHeader: true, unsafeCSS: DIFF_SURFACE_THEME_UNSAFE_CSS }} /></DiffWorkerPoolProvider></div>;
}
