import { FileDiff } from '@pierre/diffs/react';
import { DiffWorkerPoolProvider } from '../diffs/DiffWorkerPoolProvider';
import { useDiffOptions } from '../../lib/diffPrefs';
import { getRenderablePatch, DIFF_SURFACE_THEME_UNSAFE_CSS } from '../../lib/diffRender';
import { pullRequestPatch } from '../../lib/pullRequestDiff';
import { GuidedWalkthrough } from '../GuidedWalkthrough';
import { cn } from '@/lib/utils';
import { ChevronDown, Code as CodeIcon, LoaderCircle } from 'lucide-react';
import { useQuery } from '@tanstack/react-query';
import { useEffect, useMemo, useState } from 'react';
import type { FileChangeView } from '../../api';
import { prQueries } from '../../state/pullRequests/queries';
import { useOptionalRuntime } from '../../state/RuntimeProvider';
import { Selection } from './types';
/* ---------- code pane (Files changed) ---------- */

export function CodePane({ selection }: { selection: Selection }) {
  const [guided, setGuided] = useState(false);
  const runtime = useOptionalRuntime();
  const query = useQuery(prQueries.files(runtime, selection));
  const files: FileChangeView[] | null = query.data?.files ?? null;
  const loading = query.isPending;
  const error = query.error?.message ?? null;
  useEffect(() => setGuided(false), [selection.owner, selection.repo, selection.number]);

  if (guided) return <GuidedWalkthrough key={`${selection.owner}/${selection.repo}/${selection.number}`} initialSource={{ kind: 'pr', owner: selection.owner, repo: selection.repo, number: selection.number }} onChanged={() => {}} onClose={() => setGuided(false)} />;

  if (loading) {
    return (
      <div className="flex items-center gap-2 px-6 py-6 text-[13px] text-muted-foreground">
        <LoaderCircle className="size-3.5 animate-spin text-mira-blue" />
        Loading files…
      </div>
    );
  }
  if (error) {
    return (
      <div className="mx-6 my-4 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-[13px] text-destructive">
        {error}
      </div>
    );
  }
  if (!files || files.length === 0) {
    return <div className="px-6 py-6 text-[13px] text-muted-foreground/70">No file changes.</div>;
  }
  return (
    <div className="mx-auto max-w-5xl px-4 py-4">
      <div className="mb-3 flex flex-wrap items-center gap-2 text-[12px] text-muted-foreground">
        <button type="button" className="inline-flex min-h-8 shrink-0 items-center whitespace-nowrap rounded-md border border-border px-3 py-1.5 text-xs font-medium text-foreground hover:bg-accent" onClick={() => setGuided(true)}>Walk me through it</button>
        <CodeIcon className="size-4" fill="currentColor" />
        <span>
          {files.length} file{files.length === 1 ? '' : 's'} changed
        </span>
      </div>
      <div className="flex flex-col gap-3">
        {files.map((f) => (
          <FilePatch key={f.filename} file={f} />
        ))}
      </div>
    </div>
  );
}

export function FilePatch({ file }: { file: FileChangeView }) {
  const [expanded, setExpanded] = useState(true);
  const options = useDiffOptions();
  const parsed = useMemo(() => {
    const patch = pullRequestPatch(file);
    return patch ? getRenderablePatch(patch, 'mira-pr', { compactPartialHunkOffsets: true }) : null;
  }, [file]);
  return (
    <div className="overflow-hidden rounded-md border border-border">
      <button
        onClick={() => setExpanded((v) => !v)}
        className="flex w-full items-center gap-2 border-b border-border/60 bg-secondary/40 px-3 py-1.5 text-left"
      >
        <ChevronDown
          className={cn(
            'size-3 text-muted-foreground/70 transition-transform',
            !expanded && '-rotate-90',
          )}
        />
        <span className="min-w-0 flex-1 truncate font-mono text-[12.5px] text-foreground/85">
          {file.filename}
        </span>
        <span className="text-[11.5px] text-emerald-400">+{file.additions}</span>
        <span className="text-[11.5px] text-rose-500">-{file.deletions}</span>
        <span className="rounded bg-secondary px-1.5 py-0.5 text-[10.5px] uppercase tracking-wider text-muted-foreground">
          {file.status}
        </span>
      </button>
      {expanded && (parsed?.kind === 'files' ? (
        <div className="min-w-0 overflow-auto"><DiffWorkerPoolProvider>{parsed.files.map((fileDiff, index) => <FileDiff key={index} fileDiff={fileDiff} className="diff-render-surface" options={{ ...options, disableFileHeader: true, unsafeCSS: DIFF_SURFACE_THEME_UNSAFE_CSS }} />)}</DiffWorkerPoolProvider></div>
      ) : <div className="px-3 py-3 text-xs text-muted-foreground">{file.patch === null ? 'Binary or large diff omitted by GitHub.' : 'No text patch available for this file.'}</div>)}
    </div>
  );
}
