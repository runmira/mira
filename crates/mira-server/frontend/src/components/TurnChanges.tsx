import { PREF_KEYS, useBoolPref } from '../lib/prefs';
import { useTranscriptDisclosure } from './TranscriptDisclosure';
import { AnimatePresence, m, useReducedMotion } from 'framer-motion';
import { ArrowUpRight, ChevronDown, FileCode2, Undo2 } from 'lucide-react';
import { cn } from '../lib/utils';
import type { TurnDiffSummary } from '../types';

/** File rows open the editor; the CTA opens the complete turn in the drawer. */
export function TurnChanges({ summary, onOpenFile, onReview, onUndo, busy = false }: {
  summary: TurnDiffSummary; onOpenFile: (path: string) => void;
  onReview: () => void; onUndo?: () => void; busy?: boolean;
}) {
  const systemReducedMotion = useReducedMotion();
  const [preferReducedMotion] = useBoolPref(PREF_KEYS.reduceMotion, false);
  const [more, setMore] = useTranscriptDisclosure('turn-changes-more');
  if (!summary.files.length) return null;
  const added = summary.files.reduce((sum, file) => sum + file.added, 0);
  const removed = summary.files.reduce((sum, file) => sum + file.removed, 0);
  const fileRow = (file: TurnDiffSummary['files'][number]) => {
    const slash = file.path.lastIndexOf('/');
    return <button key={file.path} type="button" aria-label={`Open file ${file.path}`} onClick={() => onOpenFile(file.path)} className="group flex w-full min-w-0 items-center gap-2.5 rounded-md px-3 py-2 text-left text-[12px] leading-5 transition-colors hover:bg-secondary/60">
      <FileCode2 aria-hidden className="size-3.5 shrink-0 text-muted-foreground/70" />
      <span className="min-w-0 flex-1 truncate" title={file.path}><span className="text-muted-foreground">{file.path.slice(0, slash + 1)}</span><span className="text-foreground">{file.path.slice(slash + 1)}</span></span>
      {file.binary ? <span className="text-xs text-muted-foreground">Binary</span> : <span className="shrink-0 whitespace-nowrap font-mono text-[11px] tabular-nums"><span className="text-emerald-600 dark:text-emerald-400">+{file.added}</span>{' '}<span className="text-red-600 dark:text-red-400">−{file.removed}</span></span>}
      <ArrowUpRight aria-hidden className="size-3 shrink-0 text-muted-foreground/40 group-hover:text-foreground" />
    </button>;
  };
  return <section aria-label="Changes in this turn" className="my-3 w-full min-w-0 overflow-hidden rounded-xl border border-border/70 bg-background">
    <header className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 px-4 pb-2 pt-3">
      <h3 className="text-[13px] font-medium text-foreground">{summary.files.length} file{summary.files.length === 1 ? '' : 's'} changed</h3>
      <span className="whitespace-nowrap font-mono text-[11px] tabular-nums"><span className="text-emerald-600 dark:text-emerald-400">+{added}</span>{' '}<span className="text-red-600 dark:text-red-400">−{removed}</span></span>
    </header>
    <div className="px-1 pb-1">{summary.files.slice(0, 3).map(fileRow)}<AnimatePresence initial={false}>{more && <m.div key="more-files" initial={{ height: 0, opacity: 0 }} animate={{ height: 'auto', opacity: 1 }} exit={{ height: 0, opacity: 0 }} transition={{ duration: systemReducedMotion || preferReducedMotion ? 0 : 0.18 }} className="overflow-hidden">{summary.files.slice(3).map(fileRow)}</m.div>}</AnimatePresence>
      {summary.files.length > 3 && <button type="button" aria-expanded={more} onClick={() => setMore(value => !value)} className="flex items-center gap-1.5 px-3 py-2 text-[12px] text-muted-foreground hover:text-foreground">{more ? 'Show fewer files' : `Show ${summary.files.length - 3} more files`}<ChevronDown className={cn('size-3 transition-transform', more && 'rotate-180')} /></button>}
    </div>
    <footer className="flex flex-wrap items-center justify-end gap-2 border-t border-border/60 bg-secondary/20 px-3 py-2.5">
      {onUndo && <button type="button" disabled={busy} onClick={onUndo} title="Preview restoring files to before this turn" className="inline-flex min-h-8 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-md px-2.5 py-1.5 text-[12px] text-muted-foreground hover:bg-secondary hover:text-foreground disabled:opacity-40"><Undo2 className="size-3.5" />Undo turn</button>}
      <button type="button" onClick={onReview} className="inline-flex min-h-8 shrink-0 items-center gap-2 whitespace-nowrap rounded-md border border-border bg-background px-3 py-1.5 text-[12px] font-medium text-foreground shadow-sm transition-colors hover:bg-secondary">View changes<ArrowUpRight className="size-3.5" /></button>
    </footer>
  </section>;
}
