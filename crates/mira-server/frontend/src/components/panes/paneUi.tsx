/** Small pieces the utility panes share, so they read as one family. */
import type { ReactNode } from 'react';
import { cn } from '@/lib/utils';

/** The strip under the tab bar: controls on the left, extras right. */
export function PaneBar({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={cn(
        'flex min-h-[37px] shrink-0 items-center gap-1.5 border-b border-border/60 px-2.5 py-1.5',
        className,
      )}
    >
      {children}
    </div>
  );
}

export function PaneIconButton({
  children,
  title,
  onClick,
  disabled,
  active,
  tone,
  className,
}: {
  children: ReactNode;
  title: string;
  onClick: () => void;
  disabled?: boolean;
  active?: boolean;
  /** `danger` reddens on hover (Stop, Clear). */
  tone?: 'danger';
  className?: string;
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      onClick={onClick}
      disabled={disabled}
      className={cn(
        'flex size-6 shrink-0 items-center justify-center rounded-md transition-colors disabled:pointer-events-none disabled:opacity-30',
        active
          ? 'bg-fg/[0.1] text-foreground'
          : tone === 'danger'
            ? 'text-muted-foreground hover:bg-red-500/10 hover:text-red-500 dark:hover:text-red-400'
            : 'text-muted-foreground hover:bg-fg/[0.06] hover:text-foreground',
        className,
      )}
    >
      {children}
    </button>
  );
}

/** A labelled button for a pane's main actions ("Run", "Send to agent"). */
export function PaneButton({
  children,
  onClick,
  disabled,
  primary,
  title,
  className,
}: {
  children: ReactNode;
  onClick: () => void;
  disabled?: boolean;
  primary?: boolean;
  title?: string;
  className?: string;
}) {
  return (
    <button
      type="button"
      title={title}
      onClick={onClick}
      disabled={disabled}
      className={cn(
        'inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md px-2.5 text-[12px] font-medium transition-colors disabled:pointer-events-none disabled:opacity-40',
        primary
          ? 'bg-mira-blue text-mira-on-accent hover:bg-mira-blue/90'
          : 'border border-border/70 text-foreground/90 hover:bg-fg/[0.05]',
        className,
      )}
    >
      {children}
    </button>
  );
}

/** Centered empty state with an optional call to action. */
export function PaneEmpty({
  icon,
  title,
  children,
  action,
}: {
  icon: ReactNode;
  title: string;
  children?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="flex h-full min-h-[220px] flex-col items-center justify-center gap-2 px-6 text-center">
      <div className="mb-1 flex size-10 items-center justify-center rounded-xl border border-border/60 bg-fg/[0.03] text-muted-foreground">
        {icon}
      </div>
      <div className="text-[13px] font-medium text-foreground/90">{title}</div>
      {children && (
        <div className="max-w-[300px] text-[12px] leading-relaxed text-muted-foreground">{children}</div>
      )}
      {action && <div className="mt-2">{action}</div>}
    </div>
  );
}

/** Uppercase section heading inside a pane. */
export function PaneSection({ title, right }: { title: string; right?: ReactNode }) {
  return (
    <div className="flex items-center gap-2 px-3 pb-1.5 pt-3">
      <span className="text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground/80">
        {title}
      </span>
      <span className="flex-1" />
      {right}
    </div>
  );
}

/** Status dot: running pulses green, failed is red, done is quiet. */
export function StatusDot({ state }: { state: 'running' | 'ok' | 'failed' | 'idle' }) {
  return (
    <span className="relative flex size-2 shrink-0">
      {state === 'running' && (
        <span className="absolute inline-flex size-full animate-ping rounded-full bg-emerald-500/60" />
      )}
      <span
        className={cn(
          'relative inline-flex size-2 rounded-full',
          state === 'running' && 'bg-emerald-500',
          state === 'ok' && 'bg-fg/30',
          state === 'failed' && 'bg-red-500',
          state === 'idle' && 'bg-fg/20',
        )}
      />
    </span>
  );
}
