/**
 * "Want a second opinion?" — offered after an external agent changes files.
 *
 * Mira can host Claude Code or Codex *and* run its own models, so it can do
 * what neither does alone: have a different model check the agent's work.
 * The review is Mira's reviewer subagent (Iris by default), run on the
 * user's provider against the working tree, and lands in the review panel.
 */
import { X } from 'lucide-react';
import { useSubagent, personaName } from '../lib/subagents';
import { SubagentFace } from './SubagentFace';

export function SecondOpinion({
  agentName,
  files,
  canReview,
  onReview,
  onAddProvider,
  onDismiss,
}: {
  /** Who made the changes (`Claude Code`). */
  agentName: string;
  /** How many files the chat has changed. */
  files: number;
  /** A provider is set up, so the reviewer has a model to run on. */
  canReview: boolean;
  onReview: () => void;
  onAddProvider: () => void;
  onDismiss: () => void;
}) {
  const reviewer = useSubagent('reviewer');
  const name = personaName(reviewer, 'reviewer');
  return (
    <div className="group/second mt-1 flex animate-fade-in items-center gap-3 rounded-xl border border-border/60 bg-white/[0.025] px-3 py-2.5">
      <SubagentFace id="reviewer" face={reviewer?.face} size={30} state="idle" />
      <div className="min-w-0 flex-1">
        <div className="text-[13px] text-foreground/90">
          Want a second opinion?
        </div>
        <div className="text-[11.5px] leading-snug text-muted-foreground">
          {canReview
            ? `${name} can check ${agentName}'s changes to ${files} file${files === 1 ? '' : 's'} with a different model.`
            : `${name} reviews with your own model — add a provider and ${name} can check ${agentName}'s work.`}
        </div>
      </div>
      {canReview ? (
        <button
          type="button"
          onClick={onReview}
          className="shrink-0 rounded-lg border border-amber-400/40 bg-amber-400/10 px-2.5 py-1.5 text-[12px] font-medium text-amber-200 transition-colors hover:bg-amber-400/20"
        >
          Ask {name}
        </button>
      ) : (
        <button
          type="button"
          onClick={onAddProvider}
          className="shrink-0 rounded-lg border border-border/70 px-2.5 py-1.5 text-[12px] text-muted-foreground transition-colors hover:text-foreground"
        >
          Add a provider
        </button>
      )}
      <button
        type="button"
        onClick={onDismiss}
        aria-label="Not now"
        title="Not now"
        className="shrink-0 rounded p-1 text-muted-foreground/50 transition-colors hover:text-foreground"
      >
        <X className="size-3.5" />
      </button>
    </div>
  );
}
