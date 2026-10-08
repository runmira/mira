import { PREF_KEYS, useBoolPref } from '../lib/prefs';
import { useTranscriptDisclosure, TranscriptSessionContext } from './TranscriptDisclosure';
import { useContext, useEffect, useState } from 'react';
import { AnimatePresence, m, useReducedMotion } from 'framer-motion';
import { getTurnFileDiff } from '../api';
import { TurnSnapshotDiff } from './diffs/LazyDiffs';
import { ChevronDown, ClipboardPlus, Undo2, X } from 'lucide-react';
import { cn } from '../lib/utils';
import type { TurnDiffSummary } from '../types';

export function TurnChanges({ summary, onOpenFile, onUndo, busy = false }: { summary: TurnDiffSummary; onOpenFile: (path: string) => void; onUndo?: () => void; busy?: boolean }) {
  const systemReducedMotion = useReducedMotion();
  const [preferReducedMotion] = useBoolPref(PREF_KEYS.reduceMotion, false);
  const reduceMotion = systemReducedMotion || preferReducedMotion;
  const [more, setMore] = useTranscriptDisclosure('turn-changes-more');
  const session = useContext(TranscriptSessionContext);
  const [selected, setSelected] = useState<string | null>(null);
  const [diff, setDiff] = useState<(import('../types').DiffPreview & { before: string; after: string }) | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    if (!selected) return;
    const controller = new AbortController(); setDiff(null); setError(null);
    void getTurnFileDiff(session, summary.text, summary.occurrence, selected, controller.signal).then(value => { if (!controller.signal.aborted) setDiff(value); }).catch(error => { if (!controller.signal.aborted) setError(error.message); });
    return () => controller.abort();
  }, [session, summary.text, summary.occurrence, selected, retry]);
  if (!summary.files.length) return null;
  const added = summary.files.reduce((sum, file) => sum + file.added, 0);
  const removed = summary.files.reduce((sum, file) => sum + file.removed, 0);
  const fileRow = (file: TurnDiffSummary['files'][number]) => {
    const slash = file.path.lastIndexOf('/');
    return <button key={file.path} type="button" aria-label={`View turn changes for ${file.path}`} onClick={() => setSelected(file.path)} className={cn('flex w-full min-w-0 items-center gap-3 px-4 py-2.5 text-left text-[13px] leading-5 transition-colors hover:bg-secondary/60', selected === file.path && 'bg-secondary/60')}>
      <span className="min-w-0 flex-1 truncate" title={file.path}><span className="text-muted-foreground">{file.path.slice(0, slash + 1)}</span><span className="text-foreground">{file.path.slice(slash + 1)}</span></span>
      {file.binary ? <span className="text-xs text-muted-foreground">Binary</span> : <span className="shrink-0 whitespace-nowrap tabular-nums"><span className="text-emerald-600 dark:text-emerald-400">+{file.added}</span>{' '}<span className="text-red-600 dark:text-red-400">−{file.removed}</span></span>}
    </button>;
  };
  return <section aria-label="Changes in this turn" className="my-2 w-full overflow-hidden rounded-xl border border-border/70 bg-background">
    <div className="flex items-center gap-3 border-b border-border/70 px-4 py-3">
      <div className="flex size-10 shrink-0 items-center justify-center rounded-xl bg-secondary/70 text-muted-foreground"><ClipboardPlus className="size-5" strokeWidth={1.7} /></div>
      <div className="min-w-0 flex-1"><p className="text-[14px] font-medium leading-5 text-foreground">Edited {summary.files.length} file{summary.files.length === 1 ? '' : 's'}</p><p className="mt-0.5 text-[12px] tabular-nums"><span className="text-emerald-600 dark:text-emerald-400">+{added}</span>{' '}<span className="text-red-600 dark:text-red-400">−{removed}</span></p></div>
      <div className="flex shrink-0 items-center gap-2">{onUndo && <button type="button" disabled={busy} onClick={onUndo} title="Preview restoring files to before this turn" className="inline-flex items-center gap-1.5 rounded-lg px-2 py-1.5 text-[13px] text-foreground transition-colors hover:bg-secondary disabled:opacity-40">Undo<Undo2 className="size-3.5" /></button>}<button type="button" onClick={() => setSelected(summary.files[0].path)} className="rounded-lg border border-border/70 px-2.5 py-1.5 text-[13px] text-foreground transition-colors hover:bg-secondary">View changes</button></div>
    </div>
    <div className="py-1">{summary.files.slice(0, 3).map(fileRow)}<AnimatePresence initial={false}>{more && <m.div key="more-files" initial={{ height: 0, opacity: 0 }} animate={{ height: 'auto', opacity: 1 }} exit={{ height: 0, opacity: 0 }} transition={{ duration: reduceMotion ? 0 : 0.22, ease: 'easeInOut' }} className="overflow-hidden">{summary.files.slice(3).map(fileRow)}</m.div>}</AnimatePresence>
      {summary.files.length > 3 && <button type="button" aria-expanded={more} onClick={() => setMore(value => !value)} className="flex items-center gap-2 px-4 py-2.5 text-[13px] text-foreground hover:text-muted-foreground">{more ? 'Show fewer files' : `Show ${summary.files.length - 3} more file${summary.files.length - 3 === 1 ? '' : 's'}`}<ChevronDown className={cn('size-3.5 transition-transform', more && 'rotate-180')} /></button>}
    </div>
    <AnimatePresence initial={false}>{selected && <m.div key="turn-diff" initial={{ height: 0, opacity: 0 }} animate={{ height: 'auto', opacity: 1 }} exit={{ height: 0, opacity: 0 }} transition={{ duration: reduceMotion ? 0 : 0.22, ease: 'easeInOut' }} className="min-w-0 overflow-hidden border-t border-border/70">
      <div className="flex flex-wrap items-center gap-2 px-4 py-2 text-xs text-muted-foreground"><select aria-label="File to review in this turn" value={selected} onChange={event => setSelected(event.target.value)} className="min-w-0 flex-1 bg-background text-foreground">{summary.files.map(file => <option key={file.path} value={file.path}>{file.path}</option>)}</select><button onClick={() => onOpenFile(selected)}>Open current file</button><button aria-label="Close turn diff" onClick={() => setSelected(null)}><X className="size-3.5" /></button></div>
      {error ? <div role="alert" className="px-4 py-2 text-xs text-muted-foreground">{error} <button onClick={() => setRetry(value => value + 1)}>Retry</button></div> : diff ? <TurnSnapshotDiff path={selected} before={diff.before} after={diff.after} /> : <p role="status" className="px-4 py-2 text-xs text-muted-foreground">Loading turn diff…</p>}
    </m.div>}</AnimatePresence>
  </section>;
}
