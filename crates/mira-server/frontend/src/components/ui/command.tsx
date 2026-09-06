import * as React from 'react';
import { Command as CommandPrimitive } from 'cmdk';
import { MagnifyingGlass } from '@phosphor-icons/react';
import { cn } from '@/lib/utils';

export const Command = React.forwardRef<
  React.ElementRef<typeof CommandPrimitive>,
  React.ComponentPropsWithoutRef<typeof CommandPrimitive>
>(({ className, ...props }, ref) => (
  <CommandPrimitive
    ref={ref}
    className={cn(
      'flex h-full w-full flex-col overflow-hidden rounded-lg bg-popover text-popover-foreground',
      className,
    )}
    {...props}
  />
));
Command.displayName = 'Command';

export const CommandInput = React.forwardRef<
  React.ElementRef<typeof CommandPrimitive.Input>,
  React.ComponentPropsWithoutRef<typeof CommandPrimitive.Input>
>(({ className, ...props }, ref) => (
  <div className="flex items-center gap-2 border-b border-border px-3" cmdk-input-wrapper="">
    <MagnifyingGlass className="size-3.5 shrink-0 text-muted-foreground" />
    <CommandPrimitive.Input
      ref={ref}
      className={cn(
        'flex h-9 w-full rounded-md bg-transparent py-2 text-sm outline-none ' +
          'placeholder:text-muted-foreground/70 disabled:cursor-not-allowed disabled:opacity-50',
        className,
      )}
      {...props}
    />
  </div>
));
CommandInput.displayName = 'CommandInput';

export const CommandList = React.forwardRef<
  React.ElementRef<typeof CommandPrimitive.List>,
  React.ComponentPropsWithoutRef<typeof CommandPrimitive.List>
>(({ className, ...props }, ref) => (
  <CommandPrimitive.List
    ref={ref}
    className={cn('max-h-[26rem] overflow-y-auto overflow-x-hidden p-1', className)}
    {...props}
  />
));
CommandList.displayName = 'CommandList';

export const CommandEmpty = React.forwardRef<
  React.ElementRef<typeof CommandPrimitive.Empty>,
  React.ComponentPropsWithoutRef<typeof CommandPrimitive.Empty>
>((props, ref) => (
  <CommandPrimitive.Empty
    ref={ref}
    className="py-6 text-center text-xs text-muted-foreground"
    {...props}
  />
));
CommandEmpty.displayName = 'CommandEmpty';

export const CommandGroup = React.forwardRef<
  React.ElementRef<typeof CommandPrimitive.Group>,
  React.ComponentPropsWithoutRef<typeof CommandPrimitive.Group>
>(({ className, ...props }, ref) => (
  <CommandPrimitive.Group
    ref={ref}
    className={cn(
      'overflow-hidden p-1 text-foreground ' +
        '[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5 ' +
        '[&_[cmdk-group-heading]]:text-[10.5px] [&_[cmdk-group-heading]]:font-semibold ' +
        '[&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wider ' +
        '[&_[cmdk-group-heading]]:text-muted-foreground',
      className,
    )}
    {...props}
  />
));
CommandGroup.displayName = 'CommandGroup';

export const CommandSeparator = React.forwardRef<
  React.ElementRef<typeof CommandPrimitive.Separator>,
  React.ComponentPropsWithoutRef<typeof CommandPrimitive.Separator>
>(({ className, ...props }, ref) => (
  <CommandPrimitive.Separator ref={ref} className={cn('my-1 h-px bg-border', className)} {...props} />
));
CommandSeparator.displayName = 'CommandSeparator';

export const CommandItem = React.forwardRef<
  React.ElementRef<typeof CommandPrimitive.Item>,
  React.ComponentPropsWithoutRef<typeof CommandPrimitive.Item>
>(({ className, ...props }, ref) => (
  <CommandPrimitive.Item
    ref={ref}
    className={cn(
      'relative flex cursor-pointer select-none items-center gap-2 rounded-md px-2 py-2 text-sm outline-none ' +
        'data-[selected=true]:bg-accent/10 data-[selected=true]:text-foreground ' +
        'aria-selected:bg-accent/10 aria-selected:text-foreground ' +
        'data-[disabled=true]:pointer-events-none data-[disabled=true]:opacity-50',
      className,
    )}
    {...props}
  />
));
CommandItem.displayName = 'CommandItem';
