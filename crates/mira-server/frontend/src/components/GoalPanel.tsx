import { useState } from 'react';
import {
  Target,
  Play,
  CheckCircle,
  XCircle,
  WarningCircle,
  Hourglass,
  MinusCircle,
  ArrowClockwise,
  X,
  CaretDown,
} from '@phosphor-icons/react';
import { cn } from '@/lib/utils';
import type { Goal, GoalStatus } from '../types';

type Props = {
  goal: Goal;
  /** True while a model turn is in flight for this session. Drives
   *  the "working…" pulse + optional activity line. Idle status
   *  (running goal, not busy) reads as "waiting for evaluator." */
  busy: boolean;
  /** Short current-activity line — e.g. the last tool that was
   *  dispatched, or an assistant-text preview. Empty string when
   *  we have nothing to show. Rendered muted so it fades into the
   *  card without shouting. */
  activity: string;
  /** Drop the standing goal — server broadcasts `goal_cleared` so this
   *  panel unmounts once the local state clears. */
  onClear: () => void;
  /** Set the same condition again with iteration count reset. Only
   *  offered on terminal states. */
  onRestart: (condition: string, maxIterations: number) => void;
};

/**
 * Prominent goal card. Renders when a `/goal` is set — any state.
 *
 * Visual priority is intentional: the goal condition is the most
 * important thing on screen while autonomy is running, so we anchor
 * the panel above the transcript, use large-ish body type for the
 * contract, and lean on the status pill's colour to signal state at
 * a glance. Design cues from Claude Code and Codex — tight
 * information density, animated status only while `active`, generous
 * padding so it doesn't feel like a warning banner.
 */
export function GoalPanel({ goal, busy, activity, onClear, onRestart }: Props) {
  const [expanded, setExpanded] = useState(true);
  const isTerminal = goal.status !== 'active';
  const meta = statusMeta(goal.status);
  const Icon = meta.icon;
  // "Running" means the goal is active AND a turn is in flight.
  // "Waiting for evaluator" is the brief lull between clean stop and
  // the evaluator's verdict. Both use the same active-status colours;
  // only the microcopy differs.
  const workingNow = goal.status === 'active' && busy;
  const waitingOnEvaluator = goal.status === 'active' && !busy && goal.iterations > 0;
  const idle = goal.status === 'active' && !busy && goal.iterations === 0;

  const progressPct = Math.min(
    100,
    Math.round((goal.iterations / Math.max(1, goal.max_iterations)) * 100),
  );

  return (
    <div className="flex justify-start">
      <div
        className={cn(
          // Base card
          'group relative w-full max-w-2xl overflow-hidden rounded-xl border shadow-sm transition-colors',
          meta.cardBorder,
          meta.cardBg,
        )}
      >
        {/* Left accent bar — chunky vertical stripe so the status is
            legible on peripheral vision. */}
        <div className={cn('absolute inset-y-0 left-0 w-[3px]', meta.accentBar)} />

        {/* Header: target icon, status pill, iterations counter, collapse toggle */}
        <div className="flex items-center gap-2.5 pl-4 pr-3 pt-3 pb-2">
          <Target
            weight="fill"
            className={cn('size-[15px] shrink-0', meta.iconColor)}
          />
          <span className="text-[13px] font-semibold tracking-tight text-foreground">
            Goal
          </span>

          <div className={cn(
            'inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11.5px] font-medium',
            meta.pillClass,
          )}>
            <Icon
              weight={goal.status === 'active' ? 'fill' : 'fill'}
              className={cn(
                'size-3',
                goal.status === 'active' && 'animate-pulse',
              )}
            />
            {meta.label}
          </div>

          <span className="text-[11.5px] tabular-nums text-muted-foreground">
            {goal.iterations}/{goal.max_iterations}
            {isTerminal ? ' iterations' : ''}
          </span>

          <div className="ml-auto flex items-center gap-1">
            <button
              type="button"
              onClick={() => setExpanded((e) => !e)}
              className="rounded-md p-1 text-muted-foreground/70 hover:bg-accent/40 hover:text-foreground"
              aria-label={expanded ? 'Collapse goal' : 'Expand goal'}
              title={expanded ? 'Collapse' : 'Expand'}
            >
              <CaretDown
                weight="bold"
                className={cn('size-3.5 transition-transform', !expanded && '-rotate-90')}
              />
            </button>
            <button
              type="button"
              onClick={onClear}
              className="rounded-md p-1 text-muted-foreground/70 hover:bg-red-500/10 hover:text-red-300"
              aria-label="Clear goal"
              title="Clear goal"
            >
              <X weight="bold" className="size-3.5" />
            </button>
          </div>
        </div>

        {expanded && (
          <>
            {/* Goal condition — the contract. Whitespace preserved so
                multi-line goals read cleanly. Slightly larger text than the
                surrounding chrome to reinforce hierarchy. */}
            <div className="px-4 pb-2.5 pt-0.5">
              <p className="whitespace-pre-wrap break-words text-[13.5px] leading-relaxed text-foreground/90">
                {goal.condition}
              </p>
            </div>

            {/* Live status line: shows what mira is actually doing
                right now so the panel doesn't feel frozen while the
                model streams. Three shapes:
                  - workingNow  → spinning dots + activity preview
                  - waiting     → "Evaluator is checking…" muted
                  - idle        → gentle "waiting to start" hint
                None of these render for terminal states (met / etc). */}
            {goal.status === 'active' && (
              <div className={cn(
                'mx-4 mb-2.5 flex items-center gap-2 rounded-md border px-2.5 py-1.5',
                meta.cardBorder,
                'bg-background/30',
              )}>
                {workingNow && (
                  <>
                    <WorkingDots className={meta.iconColor} />
                    <span className={cn('text-[12px] font-medium', meta.iconColor)}>
                      Working
                    </span>
                    {activity && (
                      <>
                        <span className="text-muted-foreground/50">·</span>
                        <span className="min-w-0 truncate text-[12px] text-muted-foreground">
                          {activity}
                        </span>
                      </>
                    )}
                  </>
                )}
                {waitingOnEvaluator && (
                  <>
                    <WorkingDots className={meta.iconColor} />
                    <span className={cn('text-[12px] font-medium', meta.iconColor)}>
                      Evaluator is checking…
                    </span>
                  </>
                )}
                {idle && (
                  <>
                    <span className={cn('size-1.5 rounded-full', meta.progressBar)} />
                    <span className="text-[12px] text-muted-foreground">
                      Waiting to start — send a message to kick off, or the goal will auto-run momentarily.
                    </span>
                  </>
                )}
              </div>
            )}

            {/* Progress row: a slim bar with iteration count + tick marks
                for each iteration consumed. Cheap way to encode "budget
                used" without a heavy chart. */}
            <div className="px-4 pb-2.5">
              <div className="relative h-1 w-full overflow-hidden rounded-full bg-secondary/60">
                <div
                  className={cn(
                    'absolute inset-y-0 left-0 rounded-full transition-[width] duration-500',
                    meta.progressBar,
                    goal.status === 'active' && workingNow && 'animate-pulse',
                  )}
                  style={{ width: `${progressPct}%` }}
                />
              </div>
            </div>

            {/* Evaluator's latest reason. Empty on iteration 0 (before the
                first evaluation ran). Rendered in a subtle card so it's
                clearly separate from the condition itself — the "why we're
                still going" line has to survive a quick scan. */}
            {goal.last_reason && (
              <div className="mx-4 mb-3 rounded-md border border-border/40 bg-background/40 px-2.5 py-2">
                <div className="flex items-baseline gap-1.5">
                  <span className={cn('text-[11px] font-semibold uppercase tracking-wider', meta.reasonLabel)}>
                    {reasonLabel(goal.status)}
                  </span>
                  <span className="text-[13px] text-muted-foreground/90 leading-relaxed">
                    {goal.last_reason}
                  </span>
                </div>
              </div>
            )}

            {/* Actions row. Restart only offered when terminal — restarting
                an active goal doesn't make sense (it's already going). */}
            <div className="flex items-center justify-end gap-2 border-t border-border/40 bg-background/20 px-3 py-2">
              {isTerminal && (
                <button
                  type="button"
                  onClick={() => onRestart(goal.condition, goal.max_iterations)}
                  className="inline-flex items-center gap-1.5 rounded-md border border-border/60 bg-secondary/60 px-2.5 py-1 text-[12px] font-medium text-foreground hover:bg-secondary"
                >
                  <ArrowClockwise weight="bold" className="size-3" />
                  Restart with same goal
                </button>
              )}
              <button
                type="button"
                onClick={onClear}
                className="inline-flex items-center gap-1.5 rounded-md px-2.5 py-1 text-[12px] font-medium text-muted-foreground hover:bg-accent/40 hover:text-foreground"
              >
                Clear goal
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

/** Per-status visual metadata. Kept dense — all the styling
 *  branching happens here so the render tree stays declarative. */
function statusMeta(status: GoalStatus) {
  switch (status) {
    case 'active':
      return {
        icon: Play,
        iconColor: 'text-mira-blue',
        label: 'Running',
        cardBorder: 'border-mira-blue/25',
        cardBg: 'bg-mira-blue/[0.04]',
        accentBar: 'bg-mira-blue',
        pillClass:
          'border-mira-blue/40 bg-mira-blue/[0.10] text-mira-blue',
        progressBar: 'bg-mira-blue',
        reasonLabel: 'text-mira-blue/80',
      };
    case 'met':
      return {
        icon: CheckCircle,
        iconColor: 'text-emerald-400',
        label: 'Met',
        cardBorder: 'border-emerald-500/25',
        cardBg: 'bg-emerald-500/[0.04]',
        accentBar: 'bg-emerald-500',
        pillClass:
          'border-emerald-500/40 bg-emerald-500/[0.10] text-emerald-300',
        progressBar: 'bg-emerald-500',
        reasonLabel: 'text-emerald-400/80',
      };
    case 'impossible':
      return {
        icon: XCircle,
        iconColor: 'text-red-400',
        label: 'Impossible',
        cardBorder: 'border-red-500/25',
        cardBg: 'bg-red-500/[0.04]',
        accentBar: 'bg-red-500',
        pillClass: 'border-red-500/40 bg-red-500/[0.10] text-red-300',
        progressBar: 'bg-red-500',
        reasonLabel: 'text-red-400/80',
      };
    case 'needs_user':
      return {
        icon: WarningCircle,
        iconColor: 'text-amber-400',
        label: 'Needs you',
        cardBorder: 'border-amber-500/30',
        cardBg: 'bg-amber-500/[0.05]',
        accentBar: 'bg-amber-500',
        pillClass: 'border-amber-500/40 bg-amber-500/[0.10] text-amber-300',
        progressBar: 'bg-amber-500',
        reasonLabel: 'text-amber-400/80',
      };
    case 'exhausted':
      return {
        icon: Hourglass,
        iconColor: 'text-amber-400',
        label: 'Exhausted',
        cardBorder: 'border-amber-500/25',
        cardBg: 'bg-amber-500/[0.04]',
        accentBar: 'bg-amber-500',
        pillClass: 'border-amber-500/40 bg-amber-500/[0.10] text-amber-300',
        progressBar: 'bg-amber-500',
        reasonLabel: 'text-amber-400/80',
      };
    case 'cleared':
    default:
      return {
        icon: MinusCircle,
        iconColor: 'text-muted-foreground',
        label: 'Cleared',
        cardBorder: 'border-border/40',
        cardBg: 'bg-secondary/20',
        accentBar: 'bg-border',
        pillClass: 'border-border/60 bg-secondary/40 text-muted-foreground',
        progressBar: 'bg-muted',
        reasonLabel: 'text-muted-foreground',
      };
  }
}

/** Three bouncing dots — same visual language as the transcript's
 *  <Thinking /> indicator, scaled down for the goal panel. Tailwind's
 *  `animate-thinking-bounce` keyframes live in `tailwind.config.js`. */
function WorkingDots({ className }: { className?: string }) {
  return (
    <span className={cn('inline-flex items-center gap-0.5', className)}>
      <span
        className="size-1 rounded-full bg-current"
        style={{ animation: 'thinking-bounce 1s ease-in-out infinite', animationDelay: '0ms' }}
      />
      <span
        className="size-1 rounded-full bg-current"
        style={{ animation: 'thinking-bounce 1s ease-in-out infinite', animationDelay: '150ms' }}
      />
      <span
        className="size-1 rounded-full bg-current"
        style={{ animation: 'thinking-bounce 1s ease-in-out infinite', animationDelay: '300ms' }}
      />
    </span>
  );
}

function reasonLabel(status: GoalStatus): string {
  switch (status) {
    case 'active':
      return 'Evaluator';
    case 'met':
      return '✓ Met';
    case 'impossible':
      return 'Impossible';
    case 'needs_user':
      return 'Needs you';
    case 'exhausted':
      return 'Exhausted';
    default:
      return 'Note';
  }
}
