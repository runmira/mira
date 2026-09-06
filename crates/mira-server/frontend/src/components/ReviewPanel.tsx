import { useEffect } from 'react';
import {
  CaretRight,
  CircleNotch,
  Eye,
  ShieldCheck,
  ShieldWarning,
  X,
} from '@phosphor-icons/react';
import type { ReviewFinding, ReviewProgressEvent, ReviewSeverity } from '../types';
import { cn } from '@/lib/utils';

/**
 * Slide-out right-side panel that surfaces a `mira review` run.
 *
 * Lifecycle mirrors the server-side run: opens on the first `review_started`
 * frame, ticks through `review_progress` items, and settles on either a
 * finding list (`review_result`) or an error message (`review_error`).
 * Parent owns the state and passes it in — the panel is a pure renderer.
 */

export type ReviewState = {
  runId: string;
  /** Rolling status line from stage 1 / stage 2 progress events. */
  status: string;
  /** Latest stage-2 percentage, or null before stage 2 starts. */
  progressPct: number | null;
  findings: ReviewFinding[] | null;
  error: string | null;
  /** Verdicts observed during stage 2 (kept/dropped) — shown as a running log. */
  verdicts: { title: string; kept: boolean }[];
  done: boolean;
};

export function emptyReviewState(runId: string): ReviewState {
  return {
    runId,
    status: 'Starting review…',
    progressPct: null,
    findings: null,
    error: null,
    verdicts: [],
    done: false,
  };
}

/**
 * Apply a single server frame to a review state. Pure — callers hold the
 * previous state and stash the returned next one. Isolating the transitions
 * here keeps the parent's `onMessage` switch cheap and testable.
 */
export function applyReviewEvent(
  state: ReviewState,
  event: ReviewProgressEvent,
): ReviewState {
  switch (event.kind) {
    case 'stage1_started':
      return { ...state, status: `Reading diff (${event.diff_lines} lines)…` };
    case 'stage1_completed':
      return {
        ...state,
        status: `Stage 1: ${event.total_findings} candidate finding${event.total_findings === 1 ? '' : 's'}`,
      };
    case 'stage2_started':
      return { ...state, status: `Hostile re-verify: 0/${event.total}…`, progressPct: 0 };
    case 'stage2_item': {
      const pct = Math.round((event.index / event.total) * 100);
      const verdicts = event.kept != null
        ? [...state.verdicts, { title: event.title, kept: event.kept }]
        : state.verdicts;
      return {
        ...state,
        status: `Hostile re-verify: ${event.index}/${event.total} — ${event.title}`,
        progressPct: pct,
        verdicts,
      };
    }
    case 'completed':
      return {
        ...state,
        status: `Done — ${event.kept} kept, ${event.dropped} dropped`,
        progressPct: 100,
      };
  }
}

type Props = {
  open: boolean;
  state: ReviewState | null;
  onClose: () => void;
};

export function ReviewPanel({ open, state, onClose }: Props) {
  // Close on Esc for keyboard parity with modal dialogs.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  return (
    <>
      {/* Backdrop: subtle dim; click-through-to-close */}
      <div
        onClick={onClose}
        className={cn(
          'fixed inset-0 z-40 bg-background/40 backdrop-blur-[1px] transition-opacity',
          open ? 'opacity-100' : 'pointer-events-none opacity-0',
        )}
      />
      <aside
        className={cn(
          'fixed right-0 top-0 z-50 flex h-full w-[460px] max-w-[95vw] flex-col border-l border-border bg-card shadow-2xl transition-transform',
          open ? 'translate-x-0' : 'translate-x-full',
        )}
        aria-hidden={!open}
      >
        <div className="flex items-center justify-between border-b border-border/60 px-4 py-3">
          <div className="flex items-center gap-2">
            <Eye className="size-4 text-mira-blue" />
            <span className="text-[14px] font-semibold tracking-tight">Review</span>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            aria-label="Close review"
          >
            <X className="size-4" />
          </button>
        </div>

        <div className="flex-1 min-h-0 overflow-y-auto p-4">
          {!state && <EmptyPanel />}
          {state && (state.error
            ? <ErrorPanel text={state.error} />
            : state.findings && state.done
              ? <FindingsList findings={state.findings} />
              : <ProgressPanel state={state} />
          )}
        </div>
      </aside>
    </>
  );
}

function EmptyPanel() {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 text-center text-muted-foreground">
      <Eye className="size-8 text-muted-foreground/40" />
      <div className="text-[13px]">Run <code className="rounded bg-secondary px-1">/review</code> to inspect the current diff.</div>
    </div>
  );
}

function ProgressPanel({ state }: { state: ReviewState }) {
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2 text-[13px] text-foreground">
        <CircleNotch className="size-4 animate-spin text-mira-blue" />
        <span className="min-w-0 truncate">{state.status}</span>
      </div>
      {state.progressPct != null && (
        <div className="h-1.5 w-full overflow-hidden rounded-full bg-secondary">
          <div
            className="h-full rounded-full bg-mira-blue transition-[width]"
            style={{ width: `${state.progressPct}%` }}
          />
        </div>
      )}
      {state.verdicts.length > 0 && (
        <div className="mt-2 flex flex-col gap-1.5 border-t border-border/50 pt-2">
          <div className="text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
            Verdicts so far
          </div>
          <div className="flex flex-col gap-1">
            {state.verdicts.map((v, i) => (
              <div
                key={i}
                className={cn(
                  'flex items-center gap-2 text-[12.5px]',
                  v.kept ? 'text-foreground' : 'text-muted-foreground/60',
                )}
              >
                {v.kept
                  ? <ShieldWarning className="size-3.5 text-amber-500 shrink-0" />
                  : <ShieldCheck className="size-3.5 text-emerald-500 shrink-0" />}
                <span className={cn('min-w-0 truncate', !v.kept && 'line-through')}>
                  {v.title}
                </span>
                <span className="ml-auto text-[10.5px] uppercase tracking-wider">
                  {v.kept ? 'kept' : 'dropped'}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function ErrorPanel({ text }: { text: string }) {
  return (
    <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-[13px] text-destructive">
      {text}
    </div>
  );
}

function FindingsList({ findings }: { findings: ReviewFinding[] }) {
  if (findings.length === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 text-center">
        <ShieldCheck className="size-8 text-emerald-500" />
        <div className="text-[13.5px] text-foreground">No findings.</div>
        <div className="text-[12px] text-muted-foreground">The diff looks clean after hostile re-verify.</div>
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-3">
      <div className="text-[12px] text-muted-foreground">
        {findings.length} confirmed finding{findings.length === 1 ? '' : 's'}
      </div>
      {findings.map((f, i) => (
        <FindingCard key={i} finding={f} />
      ))}
    </div>
  );
}

function FindingCard({ finding }: { finding: ReviewFinding }) {
  const color = severityClasses(finding.severity);
  const where = finding.line != null ? `${finding.file}:${finding.line}` : finding.file;
  return (
    <div className="rounded-lg border border-border/60 bg-secondary/40 p-3">
      <div className="mb-1 flex items-center gap-2">
        <span className={cn(
          'rounded px-1.5 py-0.5 text-[10.5px] font-semibold uppercase tracking-wider',
          color.chip,
        )}>
          {finding.severity}
        </span>
        <span className="text-[13.5px] font-semibold text-foreground">{finding.title}</span>
      </div>
      <div className="flex items-center gap-1 text-[11.5px] text-muted-foreground/80 font-mono">
        <CaretRight className="size-3" />
        <span className="truncate">{where}</span>
      </div>
      <div className="mt-2 whitespace-pre-wrap text-[12.5px] text-foreground/85">
        {finding.explanation}
      </div>
      {finding.suggested_fix && (
        <div className="mt-2 rounded-md border border-emerald-500/25 bg-emerald-500/[0.06] px-2 py-1.5 text-[12px] text-emerald-100/90">
          <span className="mr-1 font-semibold">fix:</span>{finding.suggested_fix}
        </div>
      )}
      {finding.verify_note && (
        <div className="mt-1.5 text-[11px] text-muted-foreground/70">
          <span className="mr-1 font-semibold">verified:</span>{finding.verify_note}
        </div>
      )}
    </div>
  );
}

function severityClasses(sev: ReviewSeverity) {
  switch (sev) {
    case 'critical': return { chip: 'bg-destructive/20 text-destructive' };
    case 'high':     return { chip: 'bg-amber-500/20 text-amber-300' };
    case 'medium':   return { chip: 'bg-mira-blue/20 text-mira-blue' };
    case 'low':      return { chip: 'bg-secondary text-muted-foreground' };
  }
}
