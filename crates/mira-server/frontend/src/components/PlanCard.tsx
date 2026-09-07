import { useEffect, useState } from 'react';
import {
  ArrowDown,
  ArrowUp,
  Check,
  CheckCircle,
  Lightbulb,
  Plus,
  Prohibit,
  Trash,
} from '@phosphor-icons/react';
import type { PlanProposal, PlanStep } from '../types';
import { cn } from '@/lib/utils';

/**
 * Inline plan card — rendered directly in the transcript as the assistant's
 * tool output. Editable while awaiting a decision; collapses to a compact
 * summary once the user approves or cancels so the conversation history
 * stays readable.
 *
 * Card states:
 *  - `decision: null`      → editable proposal + approve/cancel controls
 *  - `decision.approved`   → green "Plan approved" summary with final steps
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
    <div
      className={cn(
        'w-full max-w-[90%] overflow-hidden rounded-xl border border-mira-blue/25 bg-gradient-to-b from-mira-blue/[0.06] to-transparent shadow-[0_1px_0_hsl(0_0%_100%/0.03)_inset]',
        'animate-fade-in',
      )}
    >
      {/* header */}
      <div className="flex items-center gap-2 border-b border-mira-blue/15 bg-mira-blue/[0.04] px-4 py-2.5">
        <div className="flex size-6 shrink-0 items-center justify-center rounded-full bg-mira-blue/20 text-mira-blue">
          <Lightbulb className="size-3.5" weight="fill" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-[10.5px] font-semibold uppercase tracking-wider text-mira-blue/80">
            Proposed plan
          </div>
          <div className="truncate text-[14px] font-semibold text-foreground">{proposal.title}</div>
        </div>
        <span className="shrink-0 rounded-full bg-mira-blue/10 px-2 py-0.5 text-[10.5px] font-semibold text-mira-blue">
          {steps.length} step{steps.length === 1 ? '' : 's'}
        </span>
      </div>

      {/* steps */}
      <div className="px-4 py-3">
        <div className="flex flex-col gap-1.5">
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
          className="mt-2 inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-[12px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          <Plus className="size-3" /> Add step
        </button>
      </div>

      {/* footer */}
      <div className="flex flex-col gap-2 border-t border-mira-blue/15 bg-mira-blue/[0.03] px-4 py-2.5">
        <input
          type="text"
          value={note}
          onChange={(e) => setNote(e.target.value)}
          placeholder="Optional note (shown if you cancel)"
          className="w-full rounded-md border border-border/60 bg-background/60 px-2.5 py-1.5 text-[12px] outline-none placeholder:text-muted-foreground/50 focus:border-mira-blue/40"
        />
        <div className="flex items-center justify-between">
          <div className="text-[11px] text-muted-foreground">
            {dirty ? 'Approve will run the edited plan' : '⌘↵ to approve'}
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => onCancel(note)}
              className="inline-flex items-center gap-1 rounded-md px-2.5 py-1.5 text-[12.5px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            >
              <Prohibit className="size-3.5" />
              Cancel
            </button>
            <button
              type="button"
              onClick={() => onApprove(steps)}
              disabled={!canApprove}
              className={cn(
                'inline-flex items-center gap-1 rounded-md bg-mira-blue px-2.5 py-1.5 text-[12.5px] font-semibold text-background transition-opacity hover:opacity-90',
                !canApprove && 'opacity-40 cursor-not-allowed',
              )}
            >
              <Check className="size-3.5" weight="bold" />
              {dirty ? 'Approve with edits' : 'Approve'}
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
    <div className="group flex items-start gap-2 rounded-md border border-transparent px-1.5 py-1.5 transition-colors hover:border-border/40 hover:bg-secondary/40">
      <span className="mt-[3px] shrink-0 rounded-full bg-secondary px-1.5 py-0.5 text-[10.5px] font-semibold text-muted-foreground">
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
      className="rounded p-1 text-muted-foreground/70 transition-colors hover:bg-accent hover:text-foreground disabled:opacity-30 disabled:hover:bg-transparent"
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
        'w-full max-w-[90%] overflow-hidden rounded-xl border',
        approved
          ? 'border-emerald-500/25 bg-emerald-500/[0.03]'
          : 'border-border/60 bg-secondary/30 opacity-80',
      )}
    >
      <div
        className={cn(
          'flex items-center gap-2 border-b px-4 py-2 text-[12.5px]',
          approved ? 'border-emerald-500/15 text-emerald-400' : 'border-border/40 text-muted-foreground',
        )}
      >
        {approved ? (
          <CheckCircle className="size-4 shrink-0" weight="fill" />
        ) : (
          <Prohibit className="size-4 shrink-0" weight="fill" />
        )}
        <span className="font-semibold">
          {approved ? 'Plan approved' : 'Plan cancelled'}
        </span>
        <span className="text-muted-foreground/70">·</span>
        <span className="truncate">{proposal.title}</span>
      </div>
      <ol className="flex flex-col gap-1 px-4 py-2.5 text-[12.5px]">
        {steps.map((s, i) => (
          <li key={i} className="flex gap-2">
            <span className="mt-[1px] shrink-0 text-muted-foreground/70">{i + 1}.</span>
            <div className="min-w-0 flex-1">
              <div className={cn(!approved && 'line-through decoration-muted-foreground/40')}>
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
        <div className="border-t border-border/40 px-4 py-2 text-[11.5px] text-muted-foreground">
          <span className="font-semibold">note:</span> {decision.note}
        </div>
      )}
    </div>
  );
}
