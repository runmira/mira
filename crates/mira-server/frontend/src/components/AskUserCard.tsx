import { useEffect, useRef, useState } from 'react';
import {
  ArrowLeft,
  ArrowRight,
  Check,
  Info,
  PencilSimpleLine,
  Sparkle,
  X,
} from '@phosphor-icons/react';
import type { AskUserAnswer, AskUserProposal } from '../types';
import { cn } from '@/lib/utils';

/**
 * Inline clarifier — rendered as the assistant's `ask_user` tool output.
 * The model poses 1-4 multiple-choice questions before committing to a
 * plan; the user picks options (with a "Recommended" hint on the model's
 * preferred pick) or writes free text via a per-question "Something else"
 * affordance. On submit the whole batch flows back as one
 * `PromptResponse`.
 *
 * Visual language: flat, monochrome. State lives in *fills*, never in
 * colored strokes — the outer card is a hairline neutral outline, each
 * option tile is a soft neutral pill that brightens on selection, and
 * the indicator is a filled/hollow dot. The only accent color anywhere
 * is the header sparkle, which acts as a wordmark rather than a stroke.
 *
 * Card states:
 *  - `decision: null`      → editable question set + submit/dismiss controls
 *  - `decision.answers`    → compact read-only summary with the user's picks
 *  - `decision.cancelled`  → dimmed "User skipped" summary
 */

export type AskUserDecision =
  | { cancelled: false; answers: AskUserAnswer[] }
  | { cancelled: true };

type Props = {
  proposal: AskUserProposal;
  decision: AskUserDecision | null;
  onSubmit: (answers: AskUserAnswer[]) => void;
  onCancel: () => void;
};

type Draft = {
  /** Selected option labels for this question (single-value list for
   *  radio questions, multi-value list for checkbox questions). */
  picked: string[];
  /** When set, the user is using the free-text "something else" path.
   *  `''` = box open but empty; `undefined` = closed. */
  custom: string | undefined;
};

export function AskUserCard({ proposal, decision, onSubmit, onCancel }: Props) {
  const [drafts, setDrafts] = useState<Draft[]>(() => proposal.questions.map(emptyDraft));
  // One-at-a-time flow: `idx` is the question the user is currently
  // working on. Advances via the CTA once an answer is picked; goes
  // back via the `Back` chip. Submit only fires on the final CTA.
  const [idx, setIdx] = useState(0);
  // Reseed when the model resubmits (rare — same call id, updated proposal).
  useEffect(() => {
    setDrafts(proposal.questions.map(emptyDraft));
    setIdx(0);
  }, [proposal]);

  if (decision) {
    return <ResolvedCard proposal={proposal} decision={decision} />;
  }

  function updateDraft(i: number, patch: (d: Draft) => Draft) {
    setDrafts((prev) => prev.map((d, j) => (j === i ? patch(d) : d)));
  }

  const total = proposal.questions.length;
  const clampedIdx = Math.min(idx, total - 1);
  const isLast = clampedIdx === total - 1;
  const currentDraft = drafts[clampedIdx] ?? emptyDraft();
  const currentReady =
    currentDraft.picked.length > 0 ||
    (currentDraft.custom ?? '').trim().length > 0;

  function advance() {
    if (!currentReady) return;
    if (!isLast) {
      setIdx((i) => Math.min(i + 1, total - 1));
      return;
    }
    // Final answer — bundle everything and hand back to the parent.
    const answers: AskUserAnswer[] = drafts.map((d) => ({
      picked: d.picked,
      custom: (d.custom ?? '').trim() ? d.custom!.trim() : null,
    }));
    onSubmit(answers);
  }

  function goBack() {
    setIdx((i) => Math.max(0, i - 1));
  }

  return (
    <div className="w-full max-w-[78%] overflow-hidden rounded-2xl border border-border/40 bg-card/80 backdrop-blur">
      {/* Header — sparkle wordmark + progress. Position-based ("1 of 3")
          because we're paginating; the count now tells the user which
          question they're on, not how many they've answered. */}
      <div className="flex items-center gap-2.5 px-4 pt-3.5 pb-3">
        <Sparkle weight="fill" className="size-3.5 text-mira-blue" />
        <span className="text-[12.5px] font-semibold tracking-tight text-foreground">
          Mira needs your input
        </span>
        <span className="ml-auto text-[11px] tabular-nums text-muted-foreground">
          {clampedIdx + 1} of {total}
        </span>
      </div>

      {/* Only the current question is mounted so the user's eye is
          always on exactly one decision. The parent card keyframes the
          idx swap for a subtle transition. */}
      <div className="border-t border-border/30">
        <QuestionRow
          key={clampedIdx}
          question={proposal.questions[clampedIdx]}
          draft={currentDraft}
          update={(patch) => updateDraft(clampedIdx, patch)}
        />
      </div>

      {/* Footer — back chip on the left (from Q2 onward), skip + CTA
          on the right. CTA label swaps to "Send answers" on the last
          question so the user knows the final tap commits. */}
      <div className="flex items-center gap-1.5 border-t border-border/30 bg-background/30 px-4 py-2.5">
        {clampedIdx > 0 ? (
          <button
            type="button"
            onClick={goBack}
            className="inline-flex items-center gap-1 rounded-md px-2.5 py-1.5 text-[11.5px] font-medium text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
          >
            <ArrowLeft className="size-3" weight="bold" />
            Back
          </button>
        ) : (
          <span />
        )}
        <button
          type="button"
          onClick={onCancel}
          className="ml-auto rounded-md px-2.5 py-1.5 text-[11.5px] font-medium text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
        >
          Skip
        </button>
        <button
          type="button"
          onClick={advance}
          disabled={!currentReady}
          className={cn(
            'inline-flex items-center gap-1.5 rounded-full px-3.5 py-1.5 text-[11.5px] font-semibold transition-all',
            currentReady
              ? 'bg-foreground text-background hover:brightness-95'
              : 'cursor-not-allowed bg-secondary/60 text-muted-foreground',
          )}
        >
          {isLast ? 'Send answers' : 'Next'}
          <ArrowRight className="size-3" weight="bold" />
        </button>
      </div>
    </div>
  );
}

function emptyDraft(_q?: unknown): Draft {
  return { picked: [], custom: undefined };
}

function QuestionRow({
  question,
  draft,
  update,
}: {
  question: AskUserProposal['questions'][number];
  draft: Draft;
  update: (patch: (d: Draft) => Draft) => void;
}) {
  const customRef = useRef<HTMLTextAreaElement | null>(null);
  const multi = question.multi_select === true;
  const customOpen = draft.custom !== undefined;

  function toggleOption(label: string) {
    update((d) => {
      // Picking any option closes the free-text path.
      const next: Draft = { ...d, custom: undefined };
      if (multi) {
        next.picked = d.picked.includes(label)
          ? d.picked.filter((l) => l !== label)
          : [...d.picked, label];
      } else {
        next.picked = d.picked.includes(label) ? [] : [label];
      }
      return next;
    });
  }

  function openCustom() {
    update((d) => ({ picked: [], custom: d?.custom ?? '' }));
    // Focus on the next tick so the textarea is mounted.
    requestAnimationFrame(() => customRef.current?.focus());
  }
  function closeCustom() {
    update((d) => ({ ...d, custom: undefined }));
  }
  function setCustomValue(v: string) {
    update(() => ({ picked: [], custom: v }));
  }

  return (
    <div className="flex flex-col gap-2.5 px-4 py-3.5">
      {question.header && (
        <div className="text-[10px] font-semibold uppercase tracking-[0.11em] text-muted-foreground/80">
          {question.header}
        </div>
      )}
      <div className="text-[13.5px] font-medium leading-snug text-foreground">
        {question.question}
      </div>

      <div className="mt-0.5 flex flex-col gap-1.5">
        {question.options.map((opt) => {
          const active = draft.picked.includes(opt.label);
          return (
            <button
              key={opt.label}
              type="button"
              onClick={() => toggleOption(opt.label)}
              className={cn(
                'group flex items-start gap-3 rounded-xl px-3 py-2.5 text-left transition-colors',
                active
                  ? 'bg-white text-black'
                  : 'bg-secondary/40 hover:bg-secondary/60',
              )}
            >
              <Indicator active={active} multi={multi} />
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <div className="flex items-center gap-1.5">
                  <span
                    className={cn(
                      'text-[13px] font-medium',
                      active ? 'text-black' : 'text-foreground',
                    )}
                  >
                    {opt.label}
                  </span>
                  {opt.recommended && (
                    <span
                      className={cn(
                        'text-[10px] font-medium uppercase tracking-wider',
                        active ? 'text-black/60' : 'text-muted-foreground/80',
                      )}
                    >
                      · Recommended
                    </span>
                  )}
                </div>
                {opt.description && (
                  <span
                    className={cn(
                      'text-[11.5px] leading-snug',
                      active ? 'text-black/70' : 'text-muted-foreground',
                    )}
                  >
                    {opt.description}
                  </span>
                )}
              </div>
            </button>
          );
        })}

        {/* "Something else" — free-text escape hatch. Same tile shape as
            the options so it feels like a peer, just quieter. */}
        {!customOpen && (
          <button
            type="button"
            onClick={openCustom}
            className="group flex items-center gap-2.5 rounded-xl px-3 py-2 text-left text-[12px] text-muted-foreground transition-colors hover:bg-secondary/40 hover:text-foreground"
          >
            <PencilSimpleLine className="size-3.5" />
            <span>Something else</span>
          </button>
        )}
        {customOpen && (
          <div className="rounded-xl bg-secondary/60 p-2.5">
            <div className="flex items-center gap-1.5 pb-1.5 text-[10px] font-semibold uppercase tracking-[0.11em] text-muted-foreground">
              <PencilSimpleLine className="size-3" />
              <span>Free response</span>
              <button
                type="button"
                onClick={closeCustom}
                className="ml-auto rounded p-0.5 text-muted-foreground/70 transition-colors hover:bg-secondary hover:text-foreground"
                aria-label="Close free-text"
              >
                <X className="size-3" />
              </button>
            </div>
            <textarea
              ref={customRef}
              value={draft.custom ?? ''}
              onChange={(e) => setCustomValue(e.target.value)}
              rows={2}
              placeholder="Do it a different way — describe what you want instead"
              className="min-h-[3rem] w-full resize-y border-0 bg-transparent p-0 text-[12.5px] leading-snug text-foreground outline-none placeholder:text-muted-foreground/50"
            />
          </div>
        )}
      </div>
    </div>
  );
}

/** Filled/hollow dot for radio, filled/hollow square for checkbox. All
 *  color lives in fills — no stroke that changes color on selection.
 *  Inactive = a muted ring; active = a solid fill with an inner
 *  contrast mark. */
function Indicator({ active, multi }: { active: boolean; multi: boolean }) {
  const shape = multi ? 'rounded-[5px]' : 'rounded-full';
  return (
    <span
      className={cn(
        'mt-[3px] flex size-[14px] shrink-0 items-center justify-center transition-colors',
        shape,
        // Selected tile is white → indicator is a filled black disc so
        // it reads as a strong "picked" mark on the light background.
        active ? 'bg-black' : 'bg-background/60 ring-1 ring-inset ring-border',
      )}
    >
      {active && multi && (
        <Check className="size-2.5 text-white" weight="bold" />
      )}
      {active && !multi && (
        <span className="size-1.5 rounded-full bg-white" />
      )}
    </span>
  );
}

/* ---------- resolved (post-decision) rendering ---------- */

function ResolvedCard({
  proposal,
  decision,
}: {
  proposal: AskUserProposal;
  decision: AskUserDecision;
}) {
  if (decision.cancelled) {
    return (
      <div className="flex w-full max-w-[78%] items-center gap-2 rounded-xl border border-border/40 bg-card/60 px-3 py-2 text-[12px] text-muted-foreground">
        <Info className="size-3.5 shrink-0" />
        <span>Question card skipped — mira will proceed without answers.</span>
      </div>
    );
  }
  return (
    <div className="flex w-full max-w-[78%] flex-col overflow-hidden rounded-xl border border-border/40 bg-card/60">
      <div className="flex items-center gap-2 border-b border-border/30 px-3.5 py-2 text-[11.5px] text-muted-foreground">
        <Check className="size-3.5 text-foreground/70" weight="bold" />
        <span>
          Answered {decision.answers.length} question
          {decision.answers.length === 1 ? '' : 's'}
        </span>
      </div>
      <ul className="flex flex-col divide-y divide-border/25">
        {proposal.questions.map((q, i) => {
          const a = decision.answers[i];
          const label = q.header ?? `Q${i + 1}`;
          const value = summarizeAnswer(a);
          return (
            <li key={i} className="flex flex-col gap-1 px-3.5 py-2.5">
              <span className="text-[9.5px] font-semibold uppercase tracking-[0.11em] text-muted-foreground/70">
                {label}
              </span>
              {/* The question itself — muted so the eye reads it as
                  context, and the answer below stands out. Without it
                  an answer like "Cutouts bucket policy" is ambiguous
                  on reload. */}
              <span className="text-[12px] leading-snug text-muted-foreground">
                {q.question}
              </span>
              <span className="text-[12.5px] font-medium text-foreground/90">
                {value}
              </span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function summarizeAnswer(a: AskUserAnswer | undefined): string {
  if (!a) return '(no answer)';
  if (a.custom && a.custom.trim().length > 0) return `“${a.custom.trim()}”`;
  if (a.picked.length > 0) return a.picked.join(', ');
  return '(skipped)';
}
