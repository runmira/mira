import { cn } from '@/lib/utils';
import {
  ArrowDown as PhArrowDown,
  ArrowRight as PhArrowRight,
  ArrowUp as PhArrowUp,
  Lightbulb as PhLightbulb,
  Plus,
  Trash,
} from 'lucide-react';
import { useEffect, useState } from 'react';
import type { PlanProposal, PlanStep } from '../../types';
/* ---------- embedded prompt components (plan / ask_user / approval) ---------- */

/** Plan proposal rendered directly inside the Composer (no outer card
 *  border — the Composer form provides the container). */
export function EmbeddedPlanCard({
  proposal,
  onApprove,
  onCancel,
}: {
  proposal: PlanProposal;
  onApprove: (steps: PlanStep[]) => void;
  onCancel: (note: string) => void;
}) {
  const [steps, setSteps] = useState<PlanStep[]>(() =>
    proposal.steps.map((s) => ({ description: s.description, why: s.why ?? undefined })),
  );
  const [note, setNote] = useState('');
  const [dirty, setDirty] = useState(false);

  useEffect(() => {
    setSteps(proposal.steps.map((s) => ({ description: s.description, why: s.why ?? undefined })));
    setDirty(false);
  }, [proposal]);

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
    <div className="flex flex-col">
      <div className="flex items-center gap-2.5 px-1.5 pt-1 pb-2">
        <PhLightbulb fill="currentColor" className="size-3.5 shrink-0 text-mira-blue" />
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

      <div className="border-t border-border/30" />

      <div className="max-h-[38vh] overflow-y-auto py-2">
        <div className="flex flex-col gap-1 px-1.5">
          {steps.map((s, i) => (
            <EmbeddedStepRow
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
          className="mx-1.5 mt-1.5 inline-flex items-center gap-1.5 rounded-full bg-fg/[0.04] ring-1 ring-fg/[0.08] px-3 py-1.5 text-[11.5px] text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
        >
          <Plus className="size-3" /> Add step
        </button>
      </div>

      <div className="border-t border-border/30" />

      <div className="flex flex-col gap-2 px-1.5 py-2.5">
        <input
          type="text"
          value={note}
          onChange={(e) => setNote(e.target.value)}
          placeholder="Optional note (shown if you cancel)"
          className="w-full rounded-md bg-secondary/50 px-2.5 py-1.5 text-[12px] outline-none placeholder:text-muted-foreground/50 focus:bg-secondary/70"
        />
        <div className="flex items-center justify-end gap-1.5">
          <span className="mr-auto text-[11px] text-muted-foreground/80">
            {dirty ? (
              'Approve will run the edited plan'
            ) : (
              <span className="touch:hidden">⌘↵ to approve</span>
            )}
          </span>
          <button
            type="button"
            onClick={() => onCancel(note)}
            className="rounded-full bg-fg/[0.04] ring-1 ring-fg/[0.08] px-4 py-1.5 text-[11.5px] font-medium touch:px-5 touch:py-2.5 touch:text-[13.5px] text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={() => onApprove(steps)}
            disabled={!canApprove}
            className={cn(
              'inline-flex items-center gap-1.5 rounded-full px-3.5 py-1.5 text-[11.5px] font-semibold touch:px-5 touch:py-2.5 touch:text-[13.5px] transition-all',
              canApprove
                ? 'bg-foreground text-background hover:brightness-95'
                : 'cursor-not-allowed bg-secondary/60 text-muted-foreground',
            )}
          >
            {dirty ? 'Approve with edits' : 'Approve'}
            <PhArrowRight className="size-3" strokeWidth={2.5} />
          </button>
        </div>
      </div>
    </div>
  );
}

export function EmbeddedStepRow({
  index,
  step,
  onChange,
  onMoveUp,
  onMoveDown,
  onRemove,
}: {
  index: number;
  step: PlanStep;
  onChange: (patch: Partial<PlanStep>) => void;
  onMoveUp?: () => void;
  onMoveDown?: () => void;
  onRemove?: () => void;
}) {
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
        <EmbeddedIconBtn onClick={onMoveUp} disabled={!onMoveUp} title="Move up">
          <PhArrowUp className="size-3" />
        </EmbeddedIconBtn>
        <EmbeddedIconBtn onClick={onMoveDown} disabled={!onMoveDown} title="Move down">
          <PhArrowDown className="size-3" />
        </EmbeddedIconBtn>
        <EmbeddedIconBtn onClick={onRemove} disabled={!onRemove} title="Remove">
          <Trash className="size-3" />
        </EmbeddedIconBtn>
      </div>
    </div>
  );
}

export function EmbeddedIconBtn({
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
