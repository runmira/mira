/**
 * The app's hover tip — the same `.tooltip` surface the toolbar buttons
 * use (terminal, panel…), with an optional shortcut tag. Wrap any button:
 *
 *   <Tip label="Allow" shortcut="Y"><button …/></Tip>
 *
 * It shows after a short delay on hover or keyboard focus, and never takes
 * the pointer, so it can't get in the way of what's under it.
 */
import type { ReactNode } from 'react';
import { cn } from '@/lib/utils';

export function Tip({
  label,
  hint,
  shortcut,
  align = 'center',
  side = 'top',
  className,
  children,
}: {
  label: ReactNode;
  /** A second, quieter line under the label. */
  hint?: ReactNode;
  /** Keyboard shortcut, shown as a tag after the label ("⌘J", "Y"). */
  shortcut?: string;
  /** Horizontal anchor: `end` for things near the right edge. */
  align?: 'start' | 'center' | 'end';
  side?: 'top' | 'bottom';
  /** Classes for the wrapper — layout the wrapped element needs from its
   *  parent (`ml-auto`, `min-w-0`) goes here. */
  className?: string;
  children: ReactNode;
}) {
  return (
    <span className={cn('group/tip relative inline-flex', className)}>
      {children}
      <span
        role="tooltip"
        className={cn(
          'tooltip pointer-events-none absolute z-40 opacity-0 transition-opacity delay-300 group-hover/tip:opacity-100 group-focus-within/tip:opacity-100',
          // One line for a label; a hint may wrap (up to the tooltip's width).
          hint ? 'tooltip-stack w-max max-w-[18rem]' : 'whitespace-nowrap',
          side === 'top' ? 'bottom-full mb-1.5' : 'top-full mt-1.5',
          align === 'center' && 'left-1/2 -translate-x-1/2',
          align === 'start' && 'left-0',
          align === 'end' && 'right-0',
        )}
      >
        {hint ? (
          <>
            <span className="inline-flex items-center gap-1.5 text-[12px] font-medium">
              {label}
              {shortcut && <span className="tooltip-tag">{shortcut}</span>}
            </span>
            <span className="text-[11px] opacity-70">{hint}</span>
          </>
        ) : (
          <>
            {label}
            {shortcut && <span className="tooltip-tag">{shortcut}</span>}
          </>
        )}
      </span>
    </span>
  );
}
