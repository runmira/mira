import { usePacedSummary } from '../lib/usePacedSummary';
import { cn } from '@/lib/utils';
import { AnimatePresence, m, useReducedMotion } from 'framer-motion';
import { ChevronDown, CircleAlert, CircleCheck, LoaderCircle } from 'lucide-react';
import { useEffect, useMemo } from 'react';
import { PREF_KEYS, useBoolPref } from '../lib/prefs';
import { STREAM_ROW_GUTTER } from '../lib/streamLayout';
import { completedToolSummary, latestToolEntry, plainToolAction } from '../lib/toolActivity';
import type { DiffPreview, ToolCall, ToolResult } from '../types';
import { ActivityShimmer } from './ActivityShimmer';
import { ToolCard, type ToolStatus } from './ToolCard';
import { infoFor } from './tools/toolInfo';
import { useTranscriptDisclosure } from './TranscriptDisclosure';
import { diffStatFor } from './tools/ToolDiff';
/** Staggered row entrance, mirroring ContextPanel's progress items:
 *  each expanded ToolCard slides in 4px from the left, fading, with a
 *  small delay relative to its position in the run. */
const itemVariants = {
  hidden: { opacity: 0, x: -4 },
  visible: (i: number) => ({
    opacity: 1,
    x: 0,
    transition: { duration: 0.16, delay: i * 0.035 },
  }),
};

/** Flat modern chip for a run of consecutive tool calls — no card border,
 *  no background box. Single row:
 *
 *      Exploring 2 reads, 4 searches                                    ✓
 *
 *  Umbrella verb mirrors Codex's Exploring/Explored pattern:
 *    - homogeneous run (all same tool) → the tool's own verb (Read/Searched/…)
 *      trailed by the target list (`foo.rs, bar.rs, +1 more`)
 *    - heterogeneous read+search only → Exploring / Explored + count summary
 *    - anything else → Working / Worked + count summary
 *
 *  Counts read as `N reads, N searches, N writes, …` in a stable order so a
 *  glance tells the user WHAT was done, not just HOW MANY things were done.
 *
 *  Click expands to reveal the individual ToolCards, tightly stacked
 *  underneath with no extra chrome. Only rendered when the run has 2+
 *  entries AND none are pending — pending calls must render individually
 *  so the y/n prompt is unmissable. */
export function ToolGroup({
  entries,
  onOpenFile,
}: {
  entries: {
    call: ToolCall;
    preview: DiffPreview | null;
    status: ToolStatus;
    result: ToolResult | null;
    progressLines?: string[];
    activityAt?: number;
  }[];
  onOpenFile?: (path: string, diff: DiffPreview | null) => void;
}) {
  const systemReducedMotion = useReducedMotion();
  const [preferReducedMotion] = useBoolPref(PREF_KEYS.reduceMotion, false);
  const reduceMotion = systemReducedMotion || preferReducedMotion;
  const [expanded, setExpanded] = useTranscriptDisclosure(`tools:${entries[0]?.call.id}`);

  const [parallel, setParallel] = useTranscriptDisclosure(`parallel:${entries[0]?.call.id}`);
  const running = entries.filter((e) => e.status === 'running').length;
  const errored = entries.filter(
    (e) => e.result?.is_error === true || e.status === 'denied',
  ).length;
  const done = entries.length - running - errored;
  const anyActive = running > 0;
  useEffect(() => {
    if (running > 1 && !parallel) setParallel(true);
  }, [running, parallel]);
  const latest = latestToolEntry(entries);
  const lastAction = anyActive
    ? latest
      ? plainToolAction(latest.call, latest.status === 'running')
      : 'Working'
    : completedToolSummary(entries);
  const nextIcon = infoFor((anyActive ? latest : entries[0])?.call.function.name ?? '').Icon;
  const displayed = usePacedSummary({ key: latest?.call.id ?? '', version: lastAction,
    value: { text: lastAction, Icon: nextIcon } }, anyActive);
  const GroupIcon = displayed.value.Icon;
  const displayAction = displayed.value.text;

  /** Net lines touched by the whole run, summed across the calls in it.
   *  Null when nothing in the group was an edit, so a run of reads or
   *  searches doesn't grow a pointless `+0 -0`. */
  const diffStat = useMemo(() => {
    let adds = 0;
    let dels = 0;
    let any = false;
    for (const e of entries) {
      const stat = diffStatFor(e.call, e.preview);
      if (!stat) continue;
      any = true;
      adds += stat.adds;
      dels += stat.dels;
    }
    return any ? { adds, dels } : null;
  }, [entries]);

  return (
    <div className="w-full">
      <button
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((v) => !v)}
        className={cn(
          'group relative flex w-full min-w-0 items-center gap-1.5 rounded-md py-1 text-left text-[13px] font-medium text-muted-foreground leading-5 transition-colors hover:bg-accent/40',
          STREAM_ROW_GUTTER,
        )}
      >
        <span aria-hidden="true" className="inline-flex size-3.5 shrink-0 items-center justify-center"><GroupIcon className="size-3.5 text-muted-foreground" /></span>
        <span className="min-w-0 flex-1 truncate" title={displayAction}>
          <ActivityShimmer active={anyActive}><span key={displayed.key} className={anyActive ? "thinking-line-enter inline-block" : undefined}>{displayAction}</span></ActivityShimmer>
        </span>
        {/* Trailing metadata reads as one group: the run count and the
            net line delta are the same kind of fact, so they share a
            size, a weight and a middot join. The delta keeps its colour —
            it is the one number on the row that says what actually
            happened — but sits at the same optical weight as the count
            beside it, so it reads as part of the row rather than a badge
            dropped onto it. */}
        {(entries.length > 1 || diffStat) && (
          <span className="flex shrink-0 items-center gap-1.5 text-[12px] tabular-nums">
            {entries.length > 1 && (
              <span className="text-muted-foreground">{entries.length} {parallel ? 'parallel' : 'calls'}</span>
            )}
            {entries.length > 1 && diffStat && <span aria-hidden className="text-muted-foreground/40">·</span>}
            {diffStat && (
              <span>
                <span className="text-emerald-400">+{diffStat.adds}</span>
                <span className="ml-1 text-rose-400">−{diffStat.dels}</span>
              </span>
            )}
          </span>
        )}
        <ChevronDown
          strokeWidth={2.5}
          className={cn(
            'size-3 shrink-0 text-foreground/70 transition-all',
            !expanded && '-rotate-90',
            !expanded && 'opacity-0 group-hover:opacity-100',
          )}
        />
        <StatusCluster running={running} done={done} errored={errored} />
      </button>

      <AnimatePresence initial={false}>
        {expanded && (
          <m.div
            initial={{ height: 0, opacity: 0 }}
            animate={{
              height: 'auto',
              opacity: 1,
              transition: { duration: reduceMotion ? 0 : 0.24, ease: [0.22, 1, 0.36, 1] },
            }}
            exit={{
              height: 0,
              opacity: 0,
              transition: { duration: reduceMotion ? 0 : 0.18, ease: 'easeInOut' },
            }}
            className="ml-6 mt-0.5 flex flex-col gap-0.5 overflow-hidden border-l border-border/40 pl-3"
          >
            {entries.map((e, i) => (
              <m.div
                key={e.call.id}
                custom={i}
                variants={itemVariants}
                // Virtual rows remount while scrolling; do not replay entrance fades.
                initial={false}
                animate="visible"
                transition={reduceMotion ? { duration: 0, delay: 0 } : undefined}
              >
                <ToolCard
                  detailsOnly={entries.length === 1 && e.call.function.name === 'bash'}
                  call={e.call}
                  preview={e.preview}
                  status={e.status}
                  result={e.result}
                  progressLines={e.progressLines}
                  onDecide={() => {}}
                  onOpenFile={onOpenFile}
                />
              </m.div>
            ))}
          </m.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/** Small pill next to the verb: `3`. Fills the same role as a count in
 *  a chat sidebar — a glance says "these belong together". Only shown for
 *  homogeneous runs; heterogeneous runs already carry per-category counts
 *  inline in the header text. */
/** Compact status trailer. Shows the strongest signal only:
 *  running (blue spinner) > errored (destructive) > all done (green tick).
 *  Keeps the row clean when everything succeeded — you shouldn't need to
 *  read a status column to know a green-check batch is fine. */
function StatusCluster({
  running,
  done,
  errored,
}: {
  running: number;
  done: number;
  errored: number;
}) {
  if (running > 0) {
    return <LoaderCircle className="ml-1 size-3 shrink-0 animate-spin text-mira-blue" />;
  }
  if (errored > 0) {
    return (
      <span
        className="ml-1 inline-flex shrink-0 items-center gap-0.5 text-[11px] text-destructive"
        title={`${errored} failed`}
      >
        <CircleAlert className="size-3" fill="currentColor" />
        {errored}
      </span>
    );
  }
  if (done > 0) {
    return (
      <span className="inline-flex" title={`${done} done`}>
        <CircleCheck className="ml-1 size-3.5 shrink-0 text-muted-foreground/60" />
      </span>
    );
  }
  return null;
}

export {
  categoryFor,
  countsByCategory,
  countsPhrase,
  infoFor,
  isExploratoryCategory,
  targetOf,
  type ToolCategory,
} from './tools/toolInfo';
