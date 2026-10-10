import { cn } from '@/lib/utils';
import { AnimatePresence, m, useReducedMotion } from 'framer-motion';
import { ChevronDown, CircleAlert, CircleCheck, LoaderCircle } from 'lucide-react';
import { useEffect, useMemo } from 'react';
import { PREF_KEYS, useBoolPref } from '../lib/prefs';
import { completedToolSummary, latestToolEntry, plainToolAction } from '../lib/toolActivity';
import type { DiffPreview, ToolCall, ToolResult } from '../types';
import { ActivityShimmer } from './ActivityShimmer';
import { ToolCard, type ToolStatus } from './ToolCard';
import {
  categoryFor,
  countsByCategory,
  countsPhrase,
  infoFor,
  isExploratoryCategory,
  targetOf,
} from './tools/toolInfo';
import { useTranscriptDisclosure } from './TranscriptDisclosure';
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
  const GroupIcon = infoFor((anyActive ? latest : entries[0])?.call.function.name ?? '').Icon;

  // Homogeneous run → use the tool's own verb + inline target list.
  // Heterogeneous run → Exploring/Working umbrella + categorized counts
  // so the reader sees "2 reads, 4 searches" instead of a mixed target
  // salad.
  const names = new Set(entries.map((e) => e.call.function.name));
  const homogeneous = names.size === 1;
  const counts = useMemo(() => countsByCategory(entries.map((e) => e.call)), [entries]);
  const exploratory = useMemo(
    () => entries.every((e) => isExploratoryCategory(categoryFor(e.call.function.name))),
    [entries],
  );
  const header = useMemo(() => {
    if (homogeneous) {
      const info = infoFor(entries[0].call.function.name);
      return { verb: anyActive ? info.verbCont : info.verbPast };
    }
    if (exploratory) {
      return { verb: anyActive ? 'Exploring' : 'Explored' };
    }
    return { verb: anyActive ? 'Working' : 'Worked' };
  }, [homogeneous, entries, anyActive, exploratory]);

  const trailingText = useMemo(() => {
    if (homogeneous)
      return previewLine(
        entries.map((e) => e.call),
        true,
      );
    const summary = countsPhrase(counts);
    return { text: summary, title: summary };
  }, [entries, homogeneous, counts]);

  return (
    <div className="w-full max-w-[78%]">
      <button
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((v) => !v)}
        className={cn(
          'group relative flex w-full min-w-0 items-center gap-1.5 rounded-md px-2 py-1 text-left text-[13px] leading-5 transition-colors hover:bg-accent/40',
        )}
      >
        <GroupIcon aria-hidden="true" className="size-3.5 shrink-0 text-muted-foreground" />
        {entries.length > 1 ? (
          <>
            <span className="min-w-0 flex-1 overflow-hidden" title={lastAction}>
              <AnimatePresence initial={false} mode="popLayout">
                <m.span
                  key={lastAction}
                  initial={{ opacity: 0, y: 4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -4 }}
                  transition={{ duration: reduceMotion ? 0 : 0.18, ease: 'easeOut' }}
                  className="block truncate"
                >
                  <ActivityShimmer active={anyActive}>{lastAction}</ActivityShimmer>
                </m.span>
              </AnimatePresence>
            </span>
            <ActivityShimmer active={anyActive} className="shrink-0 text-[11px]">
              {parallel ? 'Parallel tools' : 'Tools'}
            </ActivityShimmer>
            <CountBadge n={entries.length} />
          </>
        ) : (
          <>
            <ActivityShimmer active={anyActive} className="shrink-0">
              {header.verb}
            </ActivityShimmer>
            <span
              className={cn(
                'min-w-0 flex-1 truncate text-[13px] leading-5',
                anyActive && 'activity-shimmer animate-text-shimmer',
                // Homogeneous keeps the mono target-list look; heterogeneous
                // uses the tighter categorised phrase which reads better in
                // the UI's default sans-serif.
                homogeneous ? 'text-muted-foreground/85' : 'text-foreground/85',
              )}
              title={trailingText.title}
            >
              {trailingText.text}
            </span>
          </>
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
                initial="hidden"
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
function CountBadge({ n }: { n: number }) {
  return (
    <span className="inline-flex h-4 min-w-4 shrink-0 items-center justify-center rounded-full bg-secondary px-1.5 text-[10.5px] font-medium text-muted-foreground">
      {n}
    </span>
  );
}

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
        <CircleCheck className="ml-1 size-3.5 shrink-0 text-emerald-500" fill="currentColor" />
      </span>
    );
  }
  return null;
}

/** Build the one-line preview that trails the header. Homogeneous runs
 *  show just the targets (`foo.rs, bar.rs, +1 more`) since the umbrella
 *  verb already carries the action. Mixed runs prefix each item with its
 *  own verb (`Read foo.rs · Grepped "x" · +1 more`) so the reader can see
 *  what was actually done without expanding. */
function previewLine(calls: ToolCall[], homogeneous: boolean): { text: string; title: string } {
  if (calls.length === 0) return { text: '', title: '' };

  if (homogeneous) {
    const targets = calls.map((c) => targetOf(c)).filter(Boolean);
    if (targets.length === 0) return { text: '', title: '' };
    if (targets.length <= 2) {
      const t = targets.join(', ');
      return { text: t, title: t };
    }
    const head = targets.slice(0, 2).join(', ');
    return {
      text: `${head}, +${targets.length - 2} more`,
      title: targets.join(', '),
    };
  }

  // Mixed: `verb target` per entry, joined with a middle dot.
  const parts = calls.map((c) => {
    const info = infoFor(c.function.name);
    const target = targetOf(c);
    return target ? `${info.verbPast} ${target}` : info.verbPast;
  });
  const fullTitle = parts.join(' · ');
  if (parts.length <= 2) return { text: fullTitle, title: fullTitle };
  const head = parts.slice(0, 2).join(' · ');
  return { text: `${head} · +${parts.length - 2} more`, title: fullTitle };
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
