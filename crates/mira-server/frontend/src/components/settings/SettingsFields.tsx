import { cn } from '@/lib/utils';
import React from 'react';
export function TSection({
  icon,
  title,
  description,
  action,
  children,
}: {
  icon?: React.ReactNode;
  title: string;
  description?: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="flex flex-col gap-2.5">
      <div className="flex min-h-7 items-start justify-between gap-4 px-1">
        <div className="min-w-0">
          <h2 className="flex min-h-7 items-center gap-2 text-[13px] font-medium text-muted-foreground">
            {icon}
            {title}
          </h2>
          {description && (
            <p className="mt-1 max-w-xl text-[12px] leading-relaxed text-muted-foreground/75">
              {description}
            </p>
          )}
        </div>
        {action && <div className="flex min-h-7 shrink-0 items-center gap-1.5">{action}</div>}
      </div>
      <div
        className={cn(
          'overflow-hidden rounded-xl border border-border/60 bg-card/40',
          '[&>*+*]:border-t [&>*+*]:border-border/50',
        )}
      >
        {children}
      </div>
    </section>
  );
}

/** One setting: title + description left, compact control right. */
export function TRow({
  title,
  description,
  status,
  control,
  children,
}: {
  title: React.ReactNode;
  description?: React.ReactNode;
  status?: React.ReactNode;
  control?: React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    <div className="px-4 py-3">
      <div className="flex flex-col gap-3 sm:grid sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center sm:gap-6">
        <div className="min-w-0">
          <div className="text-[13.5px] font-medium text-foreground">{title}</div>
          {description && (
            <div className="mt-0.5 max-w-xl text-[12px] leading-relaxed text-muted-foreground/80">
              {description}
            </div>
          )}
          {status && <div className="mt-1 text-[12px] text-muted-foreground">{status}</div>}
        </div>
        {control && (
          <div className="flex min-w-0 shrink-0 items-center gap-2 sm:justify-end">{control}</div>
        )}
      </div>
      {children}
    </div>
  );
}

/** Muted footnote under a section (global caveats, file paths). */
export function TNote({ children }: { children: React.ReactNode }) {
  return (
    <p className="flex items-start gap-1.5 px-1 text-[11.5px] leading-relaxed text-muted-foreground/75">
      {children}
    </p>
  );
}

export function TSwitch({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
      className={cn(
        'relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors',
        checked ? 'bg-mira-blue' : 'bg-input',
      )}
    >
      <span
        className={cn(
          'inline-block size-4 rounded-full bg-white shadow transition-transform',
          checked ? 'translate-x-4' : 'translate-x-0.5',
        )}
      />
    </button>
  );
}
