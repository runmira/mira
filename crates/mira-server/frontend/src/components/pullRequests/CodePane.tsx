import { cn } from '@/lib/utils';
import { ChevronDown, Code as CodeIcon, LoaderCircle } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { getPullRequestFiles, type FileChangeView } from '../../api';
import { Selection } from './types';
/* ---------- code pane (Files changed) ---------- */

export function CodePane({ selection }: { selection: Selection }) {
  const [files, setFiles] = useState<FileChangeView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    setLoading(true);
    getPullRequestFiles(selection.owner, selection.repo, selection.number)
      .then((v) => {
        setFiles(v.files);
        setError(null);
      })
      .catch((e) => setError(String((e as Error).message)))
      .finally(() => setLoading(false));
  }, [selection.owner, selection.repo, selection.number]);

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
      <div className="mb-3 flex items-center gap-2 text-[12px] text-muted-foreground">
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
  const lines = useMemo(() => splitPatchLines(file.patch), [file.patch]);
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
      {expanded &&
        (file.patch === null ? (
          <div className="px-3 py-2 text-[12px] text-muted-foreground/70">
            Binary or too-large diff — omitted by GitHub.
          </div>
        ) : (
          <div className="overflow-x-auto bg-background">
            <div className="font-mono text-[12px]">
              {lines.map((l, i) => (
                <div
                  key={i}
                  className={cn(
                    'whitespace-pre px-3 py-[1px]',
                    l.kind === 'add' && 'bg-emerald-500/[0.09] text-emerald-200',
                    l.kind === 'del' && 'bg-rose-500/[0.09] text-rose-200',
                    l.kind === 'hunk' && 'bg-secondary/40 text-muted-foreground',
                    l.kind === 'ctx' && 'text-foreground/80',
                  )}
                >
                  {l.text || ' '}
                </div>
              ))}
            </div>
          </div>
        ))}
    </div>
  );
}

export type PatchLine = { kind: 'add' | 'del' | 'hunk' | 'ctx'; text: string };

export function splitPatchLines(patch: string | null): PatchLine[] {
  if (!patch) return [];
  return patch.split('\n').map((raw): PatchLine => {
    if (raw.startsWith('@@')) return { kind: 'hunk', text: raw };
    if (raw.startsWith('+')) return { kind: 'add', text: raw };
    if (raw.startsWith('-')) return { kind: 'del', text: raw };
    return { kind: 'ctx', text: raw };
  });
}
