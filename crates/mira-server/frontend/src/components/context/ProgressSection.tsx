import { AnimatePresence, m } from 'framer-motion';
import { ArrowRight, Check, Circle, LoaderCircle, Square, SquareTerminal } from 'lucide-react';
import { useEffect, useState } from 'react';
import type { BackgroundProcess } from '../../lib/backgroundProcesses';
import { personaName, useSubagents, type Subagent } from '../../lib/subagents';
import { cn } from '../../lib/utils';
import type { Entry, SubagentStreamState } from '../../transcript/entries';
import type { TaskItem } from '../../types';
import { SubagentFace } from '../SubagentFace';
import { Collapse } from '../ui/Collapse';
import { SectionHeader, itemVariants } from './SectionHeader';
/* ------------------------------------------------------------------ */
/* Helpers                                                             */
/* ------------------------------------------------------------------ */

export function fmtNum(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export function formatAgentName(raw: string): string {
  return raw
    .split(/[-_\s]+/)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(' ');
}

export function agentDisplayInfo(
  state: SubagentStreamState,
  roster: Subagent[] | undefined,
): { name: string; badge: string | null; description: string } {
  const rawCat = state.agentCategory ?? null;
  const promptStr = (state.prompt ?? '').trim();

  // The subagent's own persona, the same name its transcript row and
  // panel tab show.
  const type = state.agentName ?? null;
  const name = type
    ? personaName(
        roster?.find((r) => r.name === type),
        type,
      )
    : 'Helper';
  const badge = rawCat ? formatAgentName(rawCat) : null;
  const description = promptStr.split('\n')[0].replace(/^#+ /, '');

  return {
    name,
    badge,
    description: description.length > 44 ? description.slice(0, 44) + '…' : description,
  };
}

/* ------------------------------------------------------------------ */
/* Progress                                                            */
/* ------------------------------------------------------------------ */

/** Tasks still on the list — a deleted task was dropped, not finished. */
export function liveTasks(tasks: TaskItem[]): TaskItem[] {
  return tasks.filter((t) => t.status !== 'deleted');
}

export function ProgressSection({ tasks: all }: { tasks: TaskItem[] }) {
  const [open, setOpen] = useState(true);
  const tasks = liveTasks(all);
  const total = tasks.length;
  const done = tasks.filter((t) => t.status === 'completed').length;
  const active = tasks.find((t) => t.status === 'in_progress');

  return (
    <div>
      <SectionHeader
        label="Progress"
        right={
          <span className="text-[10.5px] text-fg/25">
            {done}/{total}
          </span>
        }
        open={open}
        onToggle={() => setOpen((v) => !v)}
      />
      <AnimatePresence initial={false}>
        {open && (
          <m.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1, transition: { duration: 0.16 } }}
            exit={{ height: 0, opacity: 0, transition: { duration: 0.12 } }}
            // -mx-2 px-2: room for the rows' full-width, rounded hover.
            className="-mx-2 overflow-hidden px-2"
          >
            <div className="flex flex-col gap-0.5 pb-1">
              <AnimatePresence>
                {tasks.map((task, i) => {
                  const isDone = task.status === 'completed';
                  const isCurrent = task.status === 'in_progress';
                  return (
                    <m.div
                      key={task.id}
                      custom={i}
                      variants={itemVariants}
                      initial="hidden"
                      animate="visible"
                      className={cn(
                        'flex items-start gap-2 text-[13px] leading-snug py-1',
                        isDone ? 'text-fg/25' : isCurrent ? 'text-fg/85' : 'text-fg/50',
                      )}
                    >
                      <span className="mt-[3px] shrink-0">
                        {isDone ? (
                          <Check className="size-3 text-green-500/60" strokeWidth={2.5} />
                        ) : isCurrent ? (
                          <ArrowRight className="size-3 text-blue-400" strokeWidth={2} />
                        ) : (
                          <Circle className="size-3 text-fg/20" />
                        )}
                      </span>
                      <span className={cn('min-w-0', isDone && 'line-through decoration-fg/15')}>
                        {task.subject}
                      </span>
                    </m.div>
                  );
                })}
              </AnimatePresence>
              {active && (
                <div className="mt-1.5 h-px bg-fg/10 rounded-full overflow-hidden">
                  <m.div
                    className="h-full bg-blue-400/40 rounded-full"
                    animate={{ x: ['-100%', '200%'] }}
                    transition={{ duration: 2, repeat: Infinity, ease: 'easeInOut' }}
                  />
                </div>
              )}
            </div>
          </m.div>
        )}
      </AnimatePresence>
    </div>
  );
}

export function BackgroundProcessesSection({
  processes,
  onOpen,
  onStop,
  stopping,
}: {
  processes: BackgroundProcess[];
  onOpen: (id: number) => void;
  onStop: (id: number) => Promise<void>;
  stopping: ReadonlySet<number>;
}) {
  const [open, setOpen] = useState(true);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => setError(null), [onStop]);
  async function stop(id: number) {
    setError(null);
    try {
      await onStop(id);
    } catch (error) {
      setError((error as Error).message);
    }
  }
  return (
    <div>
      <SectionHeader
        label="Background processes"
        open={open}
        onToggle={() => setOpen((value) => !value)}
      />
      <Collapse open={open}>
        <div className="mt-1 space-y-0.5">
          {processes.map((process) => (
            <div
              key={process.id}
              className="group flex min-w-0 items-center gap-1 rounded-md hover:bg-fg/[0.04]"
            >
              <button
                type="button"
                onClick={() => onOpen(process.id)}
                title={process.command}
                className="flex min-w-0 flex-1 items-center gap-2 rounded-md px-1.5 py-1.5 text-left text-[12px] text-fg/65 focus-visible:outline focus-visible:outline-2 focus-visible:outline-mira-blue"
              >
                <SquareTerminal
                  aria-hidden="true"
                  className="size-3.5 shrink-0 text-muted-foreground"
                />
                <span className="min-w-0 truncate">{process.command}</span>
                <span
                  aria-label="Running"
                  className="ml-auto size-1.5 shrink-0 rounded-full bg-emerald-500"
                />
              </button>
              <button
                type="button"
                aria-label={`Stop ${process.command}`}
                title="Stop process"
                disabled={stopping.has(process.id)}
                onClick={() => void stop(process.id)}
                className="mr-1 inline-flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground opacity-0 transition-opacity hover:bg-destructive/10 hover:text-destructive focus-visible:opacity-100 group-hover:opacity-100 group-focus-within:opacity-100 disabled:opacity-100 [@media(hover:none)]:opacity-100"
              >
                {stopping.has(process.id) ? (
                  <LoaderCircle aria-hidden="true" className="size-3 animate-spin" />
                ) : (
                  <Square aria-hidden="true" className="size-3" />
                )}
              </button>
            </div>
          ))}
          {error && (
            <p role="alert" className="px-1.5 py-1 text-xs text-destructive">
              {error}
            </p>
          )}
        </div>
      </Collapse>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Subagents                                                           */
/* ------------------------------------------------------------------ */

/** This chat's subagents, in transcript order. State for a call that isn't
 *  in the transcript (another chat's, or one compacted away) is left out,
 *  so the panel never shows a section with nothing in it. */
export function chatSubagents(
  subagentState: Map<string, SubagentStreamState>,
  entries: Entry[],
): { callId: string; state: SubagentStreamState }[] {
  const out: { callId: string; state: SubagentStreamState }[] = [];
  if (subagentState.size === 0) return out;
  for (const e of entries) {
    if (e.kind !== 'tool') continue;
    const s = subagentState.get(e.call.id);
    if (s) out.push({ callId: e.call.id, state: s });
  }
  return out;
}

export function SubagentsSection({
  ordered,
  onOpenAgent,
}: {
  ordered: { callId: string; state: SubagentStreamState }[];
  onOpenAgent: (callId: string) => void;
}) {
  const [open, setOpen] = useState(true);
  const roster = useSubagents()?.subagents;

  return (
    <div>
      <SectionHeader label="Subagents" open={open} onToggle={() => setOpen((v) => !v)} />
      <AnimatePresence initial={false}>
        {open && (
          <m.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1, transition: { duration: 0.16 } }}
            exit={{ height: 0, opacity: 0, transition: { duration: 0.12 } }}
            // -mx-2 px-2: room for the rows' full-width, rounded hover.
            className="-mx-2 overflow-hidden px-2"
          >
            <div className="flex flex-col gap-0.5 pb-1">
              <AnimatePresence>
                {ordered.map(({ callId, state }, i) => {
                  const type = state.agentName ?? null;
                  const persona = type ? roster?.find((r) => r.name === type) : undefined;
                  const { name, badge, description } = agentDisplayInfo(state, roster);
                  return (
                    <m.button
                      key={callId}
                      custom={i}
                      variants={itemVariants}
                      initial="hidden"
                      animate="visible"
                      type="button"
                      onClick={() => onOpenAgent(callId)}
                      className="flex items-start gap-2.5 py-1.5 -mx-2 px-2 rounded text-left hover:bg-fg/[0.04] transition-colors w-full"
                    >
                      <SubagentFace
                        id={type ?? callId}
                        face={persona?.face}
                        size={20}
                        state={state.done ? 'done' : 'working'}
                        className="mt-px"
                      />
                      <span className="flex-1 min-w-0 flex flex-col gap-0.5">
                        <span className="flex items-baseline gap-1.5 min-w-0">
                          <span className="truncate text-[13px] font-semibold text-fg/75 leading-snug">
                            {name}
                          </span>
                          {badge && (
                            <span className="shrink-0 text-[11px] font-medium text-fg/30">
                              ({badge})
                            </span>
                          )}
                        </span>
                        {description && (
                          <span className="truncate text-[12px] font-medium text-fg/40 leading-snug">
                            {description}
                          </span>
                        )}
                      </span>
                      {!state.done && (
                        <span className="text-[10px] text-fg/20 shrink-0 mt-0.5">running</span>
                      )}
                    </m.button>
                  );
                })}
              </AnimatePresence>
            </div>
          </m.div>
        )}
      </AnimatePresence>
    </div>
  );
}
