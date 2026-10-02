/**
 * A card that opens on hover (and on focus, for keyboard users) and stays
 * open while the pointer travels into it — the shared behaviour behind
 * every rich reference in a transcript.
 */
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';

export function HoverCard({
  trigger,
  children,
  onOpen,
  disabled = false,
  className,
  side = 'top',
}: {
  /** One element; it receives the hover handlers. */
  trigger: ReactNode;
  /** The card. `null` keeps it closed (e.g. nothing loaded yet). */
  children: ReactNode | null;
  /** First hover: start loading whatever the card shows. */
  onOpen?: () => void;
  disabled?: boolean;
  className?: string;
  side?: 'top' | 'bottom';
}) {
  const [open, setOpen] = useState(false);
  const openTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const clear = () => {
    if (openTimer.current) clearTimeout(openTimer.current);
    if (closeTimer.current) clearTimeout(closeTimer.current);
  };
  useEffect(() => clear, []);
  const show = () => {
    clear();
    onOpen?.();
    // A short delay, so sweeping the pointer across a paragraph doesn't
    // flash cards.
    openTimer.current = setTimeout(() => setOpen(true), 220);
  };
  const hide = () => {
    clear();
    closeTimer.current = setTimeout(() => setOpen(false), 140);
  };
  const hover = { onMouseEnter: show, onMouseLeave: hide, onFocus: show, onBlur: hide };
  return (
    <Popover open={open && !disabled && children != null} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <span className="inline" {...hover}>
          {trigger}
        </span>
      </PopoverTrigger>
      {children != null && (
        <PopoverContent
          side={side}
          align="start"
          sideOffset={6}
          onMouseEnter={() => clear()}
          onMouseLeave={hide}
          onOpenAutoFocus={(e) => e.preventDefault()}
          className={cn(
            'w-80 rounded-xl border border-border/60 bg-popover/95 p-3 text-[12.5px] shadow-xl backdrop-blur',
            className,
          )}
        >
          {children}
        </PopoverContent>
      )}
    </Popover>
  );
}
