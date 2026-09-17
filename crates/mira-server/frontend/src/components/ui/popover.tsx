import * as React from 'react';
import * as PopoverPrimitive from '@radix-ui/react-popover';
import { cn } from '@/lib/utils';

export const Popover = PopoverPrimitive.Root;
export const PopoverTrigger = PopoverPrimitive.Trigger;
export const PopoverAnchor = PopoverPrimitive.Anchor;

export const PopoverContent = React.forwardRef<
  React.ElementRef<typeof PopoverPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof PopoverPrimitive.Content>
>(({ className, align = 'end', sideOffset = 6, ...props }, ref) => (
  <PopoverPrimitive.Portal>
    <PopoverPrimitive.Content
      ref={ref}
      align={align}
      sideOffset={sideOffset}
      className={cn(
        // Elevated surface — brighter border so the boundary is
        // visible against the near-black background, tighter ring so
        // the whole popover has a clear silhouette even without the
        // heavy drop shadow.
        'z-50 w-72 rounded-lg border border-border bg-popover text-popover-foreground outline-none ' +
          'shadow-[0_20px_50px_-12px_rgba(0,0,0,0.85),0_0_0_1px_rgba(255,255,255,0.04)] ring-1 ring-white/10 ' +
          'data-[state=open]:animate-fade-in p-1',
        className,
      )}
      {...props}
    />
  </PopoverPrimitive.Portal>
));
PopoverContent.displayName = 'PopoverContent';
