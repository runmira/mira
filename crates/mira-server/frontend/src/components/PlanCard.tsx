import { useEffect, useState } from 'react';
import {
  ArrowDown,
  ArrowRight,
  ArrowUp,
  Check,
  Info,
  Lightbulb,
  Plus,
  Trash,
} from '@phosphor-icons/react';
import type { PlanProposal, PlanStep } from '../types';
import { cn } from '@/lib/utils';

/**
 * Inline plan card — rendered directly in the transcript as the assistant's
 * `plan` tool output. Editable while awaiting a decision; collapses to a
 * compact summary once the user approves or cancels so the conversation
 * history stays readable.
 *
 * Visual language matches AskUserCard: flat, monochrome, no colored
 * strokes. State lives in fills and hairline dividers. The only accent
 * color is the header lightbulb — a wordmark, not a stroke — and the
 * capsule "Approve" CTA is a solid `bg-foreground` pill.
 *
 * Card states:
 *  - `decision: null`      → editable proposal + approve/cancel controls
 *  - `decision.approved`   → neutral "Plan approved" summary with final steps
 *  - !decision.approved    → dimmed "Plan cancelled" summary + note
 */

type Decision = { approved: boolean; steps?: PlanStep[]; note?: string };

type Props = {
  proposal: PlanProposal;
  decision: Decision | null;
  onApprove: (finalSteps: PlanStep[]) => void;
  onCancel: (note: string) => void;
};

export function PlanCard({ proposal, decision, onApprove, onCancel }: Props) {
  const [steps, setSteps] = useState<PlanStep[]>(() =>
    proposal.steps.map((s) => ({ description: s.description, why: s.why ?? undefined })),
  );
  const [note, setNote] = useState('');
  const [dirty, setDirty] = useState(false);

  // If the model resubmits (rare — same call id, updated proposal), reseed
  // the local editor. Keyed on the proposal's identity to be safe.
  useEffect(() => {
    setSteps(proposal.steps.map((s) => ({ description: s.description, why: s.why ?? undefined })));
    setDirty(false);
  }, [proposal]);

  // Resolved: render a compact read-only summary.
  if (decision) {
    return <ResolvedPlan proposal={proposal} decision={decision} />;
  }

  function updateStep(i: number, patch: Partial<PlanStep>) {
    setSteps((prev) => prev.map((s, idx) => (idx === i ? { ...s, ...patch } : s)));
    setDirty(true);
  }
  function move(i: number, dir: -1 | 1) {
    const j = i + dir;
    if (j < 0 || j >= steps.length) return;
    setSteps((prev) => {
      const next = [...prev];
      [next[i], next[j]] = [next[j], next[i]];
      return next;
    });
    setDirty(true);
  }
  function remove(i: number) {
    setSteps((prev) => prev.filter((_, idx) => idx !== i));
    setDirty(true);
  }
  function add() {
    setSteps((prev) => [...prev, { description: '' }]);
    setDirty(true);
  }

  const canApprove = steps.length > 0 && steps.every((s) => s.description.trim().length > 0);

  return (
    <div className="w-full max-w-[90%] overflow-hidden rounded-2xl border border-border/40 bg-card/80 backdrop-blur animate-fade-in">
      {/* Header — lightbulb wordmark + plan title + step count. */}
      <div className="flex items-center gap-2.5 px-4 pt-3.5 pb-3">
        <Lightbulb weight="fill" className="size-3.5 text-mira-blue" />
        <div className="min-w-0 flex-1">
          <div className="text-[10px] font-semibold uppercase tracking-[0.11em] text-muted-foreground/80">
            Proposed plan
          </div>
          <div className="truncate text-[13.5px] font-semibold text-foreground">
            {proposal.title}
          </div>
        </div>
        <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground">
          {steps.length} step{steps.length === 1 ? '' : 's'}
        </span>
      </div>

      {/* Steps — hairline divider above, each row as a flat tile. */}
      <div className="border-t border-border/30 px-3 py-3">
        <div className="flex flex-col gap-1">
          {steps.map((s, i) => (
            <StepRow
              key={i}
              index={i}
              step={s}
              onChange={(patch) => updateStep(i, patch)}
              onMoveUp={i === 0 ? undefined : () => move(i, -1)}
              onMoveDown={i === steps.length - 1 ? undefined : () => move(i, 1)}
              onRemove={steps.length === 1 ? undefined : () => remove(i)}
            />
          ))}
        </div>
        <button
          type="button"
          onClick={add}
          className="mt-1.5 inline-flex items-center gap-1.5 rounded-md px-2 py-1.5 text-[11.5px] text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
        >
          <Plus className="size-3" /> Add step
        </button>
      </div>

      {/* Footer — note field + cancel/approve. Same monochrome CTA
          language as AskUserCard: filled `bg-foreground` capsule for
          the primary action, quiet ghost for cancel. */}
      <div className="flex flex-col gap-2 border-t border-border/30 bg-background/30 px-4 py-2.5">
        <input
          type="text"
          value={note}
          onChange={(e) => setNote(e.target.value)}
          placeholder="Optional note (shown if you cancel)"
          className="w-full rounded-md bg-secondary/50 px-2.5 py-1.5 text-[12px] outline-none placeholder:text-muted-foreground/50 focus:bg-secondary/70"
        />
        <div className="flex items-center justify-between">
          <div className="text-[11px] text-muted-foreground/80">
            {dirty ? 'Approve will run the edited plan' : '⌘↵ to approve'}
          </div>
          <div className="flex items-center gap-1.5">
            <button
              type="button"
              onClick={() => onCancel(note)}
              className="rounded-md px-2.5 py-1.5 text-[11.5px] font-medium text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={() => onApprove(steps)}
              disabled={!canApprove}
              className={cn(
                'inline-flex items-center gap-1.5 rounded-full px-3.5 py-1.5 text-[11.5px] font-semibold transition-all',
                canApprove
                  ? 'bg-foreground text-background hover:brightness-95'
                  : 'cursor-not-allowed bg-secondary/60 text-muted-foreground',
              )}
            >
              {dirty ? 'Approve with edits' : 'Approve'}
              <ArrowRight className="size-3" weight="bold" />
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

/* ---------- editable step row ---------- */

type StepRowProps = {
  index: number;
  step: PlanStep;
  onChange: (patch: Partial<PlanStep>) => void;
  onMoveUp?: () => void;
  onMoveDown?: () => void;
  onRemove?: () => void;
};

function StepRow({ index, step, onChange, onMoveUp, onMoveDown, onRemove }: StepRowProps) {
  return (
    <div className="group flex items-start gap-2.5 rounded-xl bg-secondary/40 px-3 py-2 transition-colors hover:bg-secondary/60">
      <span className="mt-[3px] inline-flex size-5 shrink-0 items-center justify-center rounded-full bg-background/60 text-[10.5px] font-semibold text-muted-foreground ring-1 ring-inset ring-border">
        {index + 1}
      </span>
      <div className="min-w-0 flex-1">
        <input
          type="text"
          value={step.description}
          onChange={(e) => onChange({ description: e.target.value })}
          placeholder="What this step does"
          className="w-full bg-transparent text-[13px] outline-none placeholder:text-muted-foreground/50"
        />
        {(step.why != null || step.description) && (
          <input
            type="text"
            value={step.why ?? ''}
            onChange={(e) => onChange({ why: e.target.value || undefined })}
            placeholder="Why (optional)"
            className="mt-0.5 w-full bg-transparent text-[11.5px] text-muted-foreground outline-none placeholder:text-muted-foreground/40"
          />
        )}
      </div>
      <div className="flex items-center opacity-0 transition-opacity group-hover:opacity-100">
        <IconBtn onClick={onMoveUp} disabled={!onMoveUp} title="Move up">
          <ArrowUp className="size-3" />
        </IconBtn>
        <IconBtn onClick={onMoveDown} disabled={!onMoveDown} title="Move down">
          <ArrowDown className="size-3" />
        </IconBtn>
        <IconBtn onClick={onRemove} disabled={!onRemove} title="Remove">
          <Trash className="size-3" />
        </IconBtn>
      </div>
    </div>
  );
}

function IconBtn({
  onClick,
  disabled,
  title,
  children,
}: {
  onClick?: () => void;
  disabled?: boolean;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={title}
      className="rounded p-1 text-muted-foreground/70 transition-colors hover:bg-background/60 hover:text-foreground disabled:opacity-30 disabled:hover:bg-transparent"
    >
      {children}
    </button>
  );
}

/* ---------- resolved plan (compact read-only summary) ---------- */

function ResolvedPlan({ proposal, decision }: { proposal: PlanProposal; decision: Decision }) {
  const steps = decision.steps ?? proposal.steps;
  const approved = decision.approved;
  return (
    <div
      className={cn(
        'w-full max-w-[90%] overflow-hidden rounded-xl border border-border/40 bg-card/60',
        !approved && 'opacity-85',
      )}
    >
      <div className="flex items-center gap-2 border-b border-border/30 px-3.5 py-2 text-[11.5px] text-muted-foreground">
        {approved ? (
          <Check className="size-3.5 text-foreground/70" weight="bold" />
        ) : (
          <Info className="size-3.5 text-muted-foreground" weight="regular" />
        )}
        <span className="font-semibold text-foreground">
          {approved ? 'Plan approved' : 'Plan cancelled'}
        </span>
        <span className="text-muted-foreground/60">·</span>
        <span className="truncate">{proposal.title}</span>
      </div>
      <ol className="flex flex-col gap-1 px-3.5 py-2.5 text-[12.5px]">
        {steps.map((s, i) => (
          <li key={i} className="flex gap-2">
            <span className="mt-[1px] shrink-0 tabular-nums text-muted-foreground/70">
              {i + 1}.
            </span>
            <div className="min-w-0 flex-1">
              <div
                className={cn(
                  'text-foreground/90',
                  !approved && 'line-through decoration-muted-foreground/40',
                )}
              >
                {s.description}
              </div>
              {s.why && (
                <div className="text-[11px] text-muted-foreground/70">{s.why}</div>
              )}
            </div>
          </li>
        ))}
      </ol>
      {decision.note && !approved && (
        <div className="border-t border-border/30 px-3.5 py-2 text-[11.5px] text-muted-foreground">
          <span className="font-semibold">note:</span> {decision.note}
        </div>
      )}
    </div>
  );
}
