import * as React from 'react';
import { cn } from '@/lib/utils';

export type InputProps = React.InputHTMLAttributes<HTMLInputElement>;

export const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, type, ...props }, ref) => (
    <input
      type={type}
      ref={ref}
      className={cn(
        'flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm ' +
          'transition-colors placeholder:text-muted-foreground ' +
          'focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring ' +
          'disabled:cursor-not-allowed disabled:opacity-50',
        className,
      )}
      {...props}
    />
  ),
);
Input.displayName = 'Input';

/**
 * Input styled for the sectioned-card design in Settings. Transparent
 * background embeds cleanly on the elevated card; border stays quiet
 * until hover/focus. Pair with `Select` for a consistent look inside
 * a `SectionShell` card.
 */
export const SectionInput = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, type, ...props }, ref) => (
    <input
      type={type}
      ref={ref}
      className={cn(
        'flex h-9 w-full rounded-md border border-border/60 bg-transparent px-3 text-[13px] text-foreground outline-none transition-colors ' +
          'placeholder:text-muted-foreground/70 ' +
          'hover:border-border ' +
          'focus-visible:border-border focus-visible:ring-1 focus-visible:ring-ring ' +
          'disabled:cursor-not-allowed disabled:opacity-50',
        className,
      )}
      {...props}
    />
  ),
);
SectionInput.displayName = 'SectionInput';
