import { cn } from '@/lib/utils';
import { AnimatePresence, m } from 'framer-motion';
import {
  PenLine,
  ArrowLeft as PhArrowLeft,
  ArrowRight as PhArrowRight,
  Check as PhCheck,
  Sparkle,
  X,
} from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import type { AskUserAnswer, AskUserProposal } from '../../types';
/** ask_user proposal rendered directly inside the Composer. */
export function EmbeddedAskUserCard({
  proposal,
  onSubmit,
  onCancel,
  asker = 'Mira',
}: {
  proposal: AskUserProposal;
  /** Who is asking — Mira, or the agent driving this chat. */
  asker?: string;
  onSubmit: (answers: AskUserAnswer[]) => void;
  onCancel: () => void;
}) {
  type Draft = { picked: string[]; custom: string | undefined };
  function emptyDraft(): Draft {
    return { picked: [], custom: undefined };
  }

  const [drafts, setDrafts] = useState<Draft[]>(() => proposal.questions.map(emptyDraft));
  const [idx, setIdx] = useState(0);
  const [dir, setDir] = useState<1 | -1>(1);
  const customRef = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    setDrafts(proposal.questions.map(emptyDraft));
    setIdx(0);
    setDir(1);
  }, [proposal]);

  const total = proposal.questions.length;
  const clampedIdx = Math.min(idx, total - 1);
  const isLast = clampedIdx === total - 1;
  const currentDraft = drafts[clampedIdx] ?? emptyDraft();
  const currentReady =
    currentDraft.picked.length > 0 || (currentDraft.custom ?? '').trim().length > 0;

  function updateDraft(i: number, patch: (d: Draft) => Draft) {
    setDrafts((prev) => prev.map((d, j) => (j === i ? patch(d) : d)));
  }

  function advance() {
    if (!currentReady) return;
    if (!isLast) {
      setDir(1);
      setIdx((i) => Math.min(i + 1, total - 1));
      return;
    }
    const answers: AskUserAnswer[] = drafts.map((d) => ({
      picked: d.picked,
      custom: (d.custom ?? '').trim() ? d.custom!.trim() : null,
    }));
    onSubmit(answers);
  }

  const q = proposal.questions[clampedIdx];
  const multi = q.multi_select === true;
  const customOpen = currentDraft.custom !== undefined;

  function toggleOption(label: string) {
    updateDraft(clampedIdx, (d) => {
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
    updateDraft(clampedIdx, (d) => ({ picked: [], custom: d?.custom ?? '' }));
    requestAnimationFrame(() => customRef.current?.focus());
  }
  function closeCustom() {
    updateDraft(clampedIdx, (d) => ({ ...d, custom: undefined }));
  }

  return (
    <div className="flex flex-col">
      <div className="flex items-center gap-2.5 px-1.5 pt-1 pb-2">
        <Sparkle fill="currentColor" className="size-3.5 text-mira-blue" />
        <span className="text-[12.5px] font-semibold tracking-tight text-foreground">
          {asker} needs your input
        </span>
        <span className="ml-auto text-[11px] tabular-nums text-muted-foreground">
          {clampedIdx + 1} of {total}
        </span>
      </div>

      <div className="border-t border-border/30" />

      <AnimatePresence initial={false} mode="wait">
        <m.div
          key={clampedIdx}
          initial={{ opacity: 0, x: dir * 24 }}
          animate={{ opacity: 1, x: 0 }}
          exit={{ opacity: 0, x: dir * -24 }}
          transition={{ duration: 0.18, ease: [0.4, 0, 0.2, 1] }}
          className="flex flex-col gap-2.5 px-1.5 py-3 max-h-[45vh] overflow-y-auto"
        >
          {q.header && (
            <div className="text-[10px] font-semibold uppercase tracking-[0.11em] text-muted-foreground/80">
              {q.header}
            </div>
          )}
          <div className="text-[13.5px] font-medium leading-snug text-foreground">{q.question}</div>
          <div className="flex flex-col gap-1.5">
            {q.options.map((opt) => {
              const active = currentDraft.picked.includes(opt.label);
              return (
                <button
                  key={opt.label}
                  type="button"
                  onClick={() => toggleOption(opt.label)}
                  className={cn(
                    'group flex items-start gap-3 rounded-xl px-3 py-2.5 text-left transition-colors',
                    active
                      ? 'bg-foreground text-background'
                      : 'bg-secondary/40 hover:bg-secondary/60',
                  )}
                >
                  <span
                    className={cn(
                      'mt-[3px] flex size-[14px] shrink-0 items-center justify-center transition-colors',
                      multi ? 'rounded-[5px]' : 'rounded-full',
                      active ? 'bg-background' : 'bg-background/60 ring-1 ring-inset ring-border',
                    )}
                  >
                    {active && multi && (
                      <PhCheck className="size-2.5 text-foreground" strokeWidth={2.5} />
                    )}
                    {active && !multi && <span className="size-1.5 rounded-full bg-foreground" />}
                  </span>
                  <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <div className="flex items-center gap-1.5">
                      <span
                        className={cn(
                          'text-[13px] font-medium',
                          active ? 'text-background' : 'text-foreground',
                        )}
                      >
                        {opt.label}
                      </span>
                      {opt.recommended && (
                        <span
                          className={cn(
                            'text-[10px] font-medium uppercase tracking-wider',
                            active ? 'text-background/65' : 'text-muted-foreground/80',
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
                          active ? 'text-background/70' : 'text-muted-foreground',
                        )}
                      >
                        {opt.description}
                      </span>
                    )}
                  </div>
                </button>
              );
            })}
            {!customOpen && (
              <button
                type="button"
                onClick={openCustom}
                className="group flex items-center gap-2.5 rounded-xl px-3 py-2 text-left text-[12px] text-muted-foreground transition-colors hover:bg-secondary/40 hover:text-foreground"
              >
                <PenLine className="size-3.5" />
                <span>Something else</span>
              </button>
            )}
            {customOpen && (
              <div className="rounded-xl bg-secondary/60 p-2.5">
                <div className="flex items-center gap-1.5 pb-1.5 text-[10px] font-semibold uppercase tracking-[0.11em] text-muted-foreground">
                  <PenLine className="size-3" />
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
                  value={currentDraft.custom ?? ''}
                  onChange={(e) =>
                    updateDraft(clampedIdx, () => ({ picked: [], custom: e.target.value }))
                  }
                  rows={2}
                  placeholder="Do it a different way — describe what you want instead"
                  className="min-h-[3rem] w-full resize-y border-0 bg-transparent p-0 text-[12.5px] leading-snug text-foreground outline-none placeholder:text-muted-foreground/50"
                />
              </div>
            )}
          </div>
        </m.div>
      </AnimatePresence>

      <div className="border-t border-border/30" />

      <div className="flex items-center gap-1.5 px-1.5 py-2.5">
        {clampedIdx > 0 ? (
          <button
            type="button"
            onClick={() => {
              setDir(-1);
              setIdx((i) => Math.max(0, i - 1));
            }}
            className="inline-flex items-center gap-1 rounded-full bg-fg/[0.04] ring-1 ring-fg/[0.08] px-4 py-1.5 text-[11.5px] font-medium touch:px-5 touch:py-2.5 touch:text-[13.5px] text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
          >
            <PhArrowLeft className="size-3" strokeWidth={2.5} />
            Back
          </button>
        ) : (
          <span />
        )}
        <button
          type="button"
          onClick={onCancel}
          className="ml-auto rounded-full bg-fg/[0.04] ring-1 ring-fg/[0.08] px-4 py-1.5 text-[11.5px] font-medium touch:px-5 touch:py-2.5 touch:text-[13.5px] text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
        >
          Skip
        </button>
        <button
          type="button"
          onClick={advance}
          disabled={!currentReady}
          className={cn(
            'inline-flex items-center gap-1.5 rounded-full px-3.5 py-1.5 text-[11.5px] font-semibold touch:px-5 touch:py-2.5 touch:text-[13.5px] transition-all',
            currentReady
              ? 'bg-foreground text-background hover:brightness-95'
              : 'cursor-not-allowed bg-secondary/60 text-muted-foreground',
          )}
        >
          {isLast ? 'Send answers' : 'Next'}
          <PhArrowRight className="size-3" strokeWidth={2.5} />
        </button>
      </div>
    </div>
  );
}
