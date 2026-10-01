import type { ReactNode } from 'react';
import { AlertTriangle, ShieldQuestion } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent } from '@/components/ui/dialog';

/** One thing the user can decide. */
export type ApprovalChoice = {
  id: string;
  label: string;
  /** One-line explanation under the label, for choices that need it (e.g.
   *  what a permission posture actually grants). */
  hint?: string;
  /** Native tooltip, so a short label can still explain itself. */
  title?: string;
  /** Styles the choice as the safe/default one. */
  primary?: boolean;
  /** Styles it as the destructive one, for grants of wider authority. */
  destructive?: boolean;
  disabled?: boolean;
};

export type ApprovalRequest = {
  /** Short question, e.g. `Run a command?` */
  title: string;
  /** What is being asked, and why it matters. Shown above the choices. */
  body: ReactNode;
  choices: ApprovalChoice[];
  /** Fired when dismissed without choosing. */
  onDismiss: () => void;
  /** Fired with the chosen id. */
  onChoose: (id: string) => void;
  /**
   * Why this needs a decision at all, shown as a small lead-in. An approval
   * the user cannot place — which agent, what tool, what scope — is a prompt
   * they have to guess at.
   */
  source?: { label: string; detail?: string };
};

/**
 * The one approval surface.
 *
 * There used to be three: the composer's embedded tool-approval card, a
 * confirm dialog living under `components/plugins/` that the privileged
 * agent-mode prompt reused, and a mode-specific variant. Same decision, three
 * looks — so a user learned the buttons in one place and then had to learn
 * them again in another, and "always allow" behaved differently depending on
 * which one appeared.
 *
 * Now anything that needs a yes/no from the user — a tool call, an agent
 * asking for a wider permission mode, a plan to accept — renders through
 * here. Callers supply the choices; the layout, the dismiss behaviour and the
 * affordances are decided once.
 */
/**
 * The row of decisions, shared by every approval surface.
 *
 * Extracted so the composer's inline card and the modal cannot drift into
 * offering different sets of answers to the same question — the labels and
 * their meaning ("Deny" vs "Always allow") are a contract with the user, and
 * having them written twice is how they end up inconsistent.
 */
export function ApprovalChoices({
  choices,
  onChoose,
  size = 'sm',
}: {
  choices: ApprovalChoice[];
  onChoose: (id: string) => void;
  size?: 'sm' | 'default' | 'lg';
}) {
  return (
    // Wraps rather than overflows: in a narrow composer (side panel open)
    // a fixed row clipped the leftmost choice — usually "Deny".
    <div className="flex flex-wrap items-center justify-end gap-1.5">
      {choices.map((c) => (
        <div key={c.id} className="flex flex-col items-end gap-0.5">
          <Button
            variant={c.destructive ? 'destructive' : c.primary ? 'default' : 'ghost'}
            size={size}
            disabled={c.disabled}
            title={c.title}
            onClick={() => onChoose(c.id)}
          >
            {c.label}
          </Button>
          {c.hint && (
            <span className="max-w-[16rem] text-right text-[10.5px] leading-tight text-muted-foreground/70">
              {c.hint}
            </span>
          )}
        </div>
      ))}
    </div>
  );
}

export function ApprovalDialog({
  request,
  tone = 'default',
}: {
  request: ApprovalRequest;
  /**
   * `consequential` marks a grant of standing authority rather than a
   * one-off action, and is styled so it does not read as routine.
   */
  tone?: 'default' | 'consequential';
}) {
  const Icon = tone === 'consequential' ? ShieldQuestion : AlertTriangle;
  return (
    <Dialog open onOpenChange={(o) => !o && request.onDismiss()}>
      <DialogContent className="max-w-md">
        {request.source && (
          <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
            <span className="truncate font-medium text-foreground/80">
              {request.source.label}
            </span>
            {request.source.detail && (
              <span className="truncate opacity-70">{request.source.detail}</span>
            )}
          </div>
        )}
        <div className="flex items-start gap-2 text-[15px] font-semibold">
          <Icon
            className={
              tone === 'consequential'
                ? 'mt-0.5 size-4 shrink-0 text-mira-warn'
                : 'mt-0.5 size-4 shrink-0 text-mira-tool'
            }
          />
          <span className="min-w-0">{request.title}</span>
        </div>
        <div className="text-[13px] text-muted-foreground">{request.body}</div>
        <ApprovalChoices choices={request.choices} onChoose={request.onChoose} />
      </DialogContent>
    </Dialog>
  );
}
