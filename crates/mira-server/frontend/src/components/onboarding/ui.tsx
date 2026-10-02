/**
 * The pieces every onboarding step is built from, so the steps share one
 * look: a heading block, the footer with Back and the main action, inputs,
 * and the raised "tile" surface.
 *
 * Surfaces are tuned for the onboarding stage (an opaque, near-black card),
 * not the window: in the desktop app the window itself is transparent, and
 * anything drawn straight on it sits on the user's wallpaper.
 */
import type { ButtonHTMLAttributes, ReactNode } from 'react';
import { ArrowLeft, ArrowRight, Loader2 } from 'lucide-react';
import { cn } from '@/lib/utils';

/** A raised surface inside the stage: visible on the card, quiet at rest. */
export const tile = 'rounded-2xl border border-fg/[0.08] bg-fg/[0.035]';
/** The same, for things you can click. */
export const tileButton =
  'rounded-2xl border border-fg/[0.08] bg-fg/[0.035] transition-[background-color,border-color,box-shadow] hover:border-fg/[0.16] hover:bg-fg/[0.06]';
/** Text inputs. */
export const field =
  'h-11 w-full rounded-xl border border-fg/[0.1] bg-shade/40 px-3.5 text-[14px] text-foreground outline-none transition-[border-color,box-shadow] placeholder:text-muted-foreground/45 focus:border-mira-blue/70 focus:ring-4 focus:ring-mira-blue/15';

export function StepHeader({ eyebrow, title, children }: { eyebrow?: string; title: ReactNode; children?: ReactNode }) {
  return (
    <header className="flex flex-col gap-2.5">
      {eyebrow && (
        <span className="text-[11px] font-semibold uppercase tracking-[0.14em] text-mira-blue/90">{eyebrow}</span>
      )}
      <h1 className="text-balance text-[26px] font-semibold leading-[1.15] tracking-[-0.022em] text-foreground">
        {title}
      </h1>
      {children && <p className="max-w-[62ch] text-[14px] leading-relaxed text-muted-foreground">{children}</p>}
    </header>
  );
}

export function PrimaryButton({
  pending,
  children,
  className,
  arrow = true,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { pending?: boolean; arrow?: boolean }) {
  return (
    <button
      type="button"
      disabled={pending || rest.disabled}
      {...rest}
      className={cn(
        'group inline-flex h-10 items-center justify-center gap-2 rounded-xl bg-mira-blue px-5 text-[13.5px] font-semibold text-mira-on-accent',
        'shadow-[0_0_0_1px_rgba(122,162,247,0.5),0_8px_24px_-8px_rgba(122,162,247,0.55)] transition-[filter,opacity,transform]',
        'hover:brightness-110 active:translate-y-px disabled:pointer-events-none disabled:opacity-45',
        className,
      )}
    >
      {pending && <Loader2 className="size-4 animate-spin" />}
      {children}
      {arrow && !pending && <ArrowRight className="size-4 transition-transform group-hover:translate-x-0.5" />}
    </button>
  );
}

export function GhostButton({ children, className, ...rest }: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      {...rest}
      className={cn(
        'inline-flex h-10 items-center gap-1.5 rounded-xl px-3.5 text-[13px] text-muted-foreground transition-colors hover:bg-fg/[0.05] hover:text-foreground disabled:opacity-50',
        className,
      )}
    >
      {children}
    </button>
  );
}

/** The bottom row of a step: Back on the left, a status line, actions on the right. */
export function StepFooter({ onBack, status, children }: { onBack?: () => void; status?: ReactNode; children: ReactNode }) {
  return (
    <footer className="mt-2 flex items-center gap-3 border-t border-fg/[0.06] pt-5">
      {onBack && (
        <GhostButton onClick={onBack} className="-ml-2 pl-2.5">
          <ArrowLeft className="size-4" />
          Back
        </GhostButton>
      )}
      <span className="min-w-0 flex-1 truncate text-[12.5px] text-muted-foreground">{status}</span>
      {children}
    </footer>
  );
}

export function ErrorText({ children }: { children: ReactNode }) {
  return (
    <p className="rounded-xl border border-red-500/20 bg-red-500/[0.07] px-3 py-2 text-[12.5px] text-red-300">{children}</p>
  );
}
