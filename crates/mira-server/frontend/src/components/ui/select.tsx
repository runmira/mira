import * as React from 'react';
import { CaretDown, Check } from '@phosphor-icons/react';
import { Popover, PopoverContent, PopoverTrigger } from './popover';
import { cn } from '@/lib/utils';

/**
 * Custom Select. Built on Radix Popover so it inherits proper
 * portalling, focus trapping, and outside-click semantics; the
 * trigger and options list are pure React so we get full control
 * over styling to match the sectioned-card design.
 *
 * We deliberately don't use `@radix-ui/react-select` — its
 * accessibility model is stronger but it fights hard for its own
 * styling and adds a dependency. Popover-based selects are the same
 * pattern shadcn's `Combobox` recipe uses.
 *
 * Options render inside a Portal (via PopoverContent) so the list
 * can escape parent overflow: hidden. Content width matches the
 * trigger via `--radix-popover-trigger-width`, which Radix sets
 * automatically on the trigger element.
 */

export type SelectOption<T extends string = string> = {
  value: T;
  label: string;
  /** Optional muted subtext shown under the label in the dropdown
   *  (not in the trigger). Great for base URLs, model ids, etc. */
  hint?: string;
  /** Optional trailing pill rendered on the dropdown row — used e.g. to
   *  mark providers that support OAuth sign-in so users can see the
   *  affordance without picking each provider first. */
  badge?: string;
};

type SelectProps<T extends string = string> = {
  value: T;
  onChange: (v: T) => void;
  options: SelectOption<T>[];
  placeholder?: string;
  disabled?: boolean;
  /** Extra classes for the trigger button. */
  className?: string;
  /** Match the trigger's width by default. Pass `false` to let the
   *  popover size to its content — useful when options are longer
   *  than the trigger. */
  matchTriggerWidth?: boolean;
};

export function Select<T extends string = string>({
  value,
  onChange,
  options,
  placeholder,
  disabled,
  className,
  matchTriggerWidth = true,
}: SelectProps<T>) {
  const [open, setOpen] = React.useState(false);
  const current = options.find((o) => o.value === value);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          disabled={disabled}
          className={cn(
            // Section-flavored trigger: transparent bg (embeds cleanly
            // in the elevated card), quiet border that brightens on
            // hover/focus. Mirrors the SectionInput styling so a
            // Select and Input side-by-side in the same card look
            // like siblings.
            'flex h-9 w-full items-center justify-between gap-2 rounded-md border border-border/60 bg-transparent px-3 text-[13px] text-foreground outline-none transition-colors',
            'hover:border-border focus-visible:ring-1 focus-visible:ring-ring',
            'disabled:cursor-not-allowed disabled:opacity-50',
            'data-[state=open]:border-border data-[state=open]:ring-1 data-[state=open]:ring-ring/40',
            className,
          )}
        >
          <span
            className={cn(
              'min-w-0 flex-1 truncate text-left',
              !current && 'text-muted-foreground',
            )}
          >
            {current?.label ?? placeholder ?? '—'}
          </span>
          <CaretDown className="size-3 shrink-0 text-muted-foreground" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        sideOffset={4}
        className={cn(
          'max-h-72 overflow-y-auto p-1',
          matchTriggerWidth && 'w-[--radix-popover-trigger-width]',
        )}
      >
        {options.length === 0 ? (
          <div className="px-2.5 py-2 text-[12.5px] text-muted-foreground">
            No options.
          </div>
        ) : (
          options.map((o) => {
            const selected = o.value === value;
            return (
              <button
                key={String(o.value)}
                type="button"
                onClick={() => {
                  onChange(o.value);
                  setOpen(false);
                }}
                className={cn(
                  'flex w-full items-start gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors',
                  'hover:bg-accent hover:text-foreground',
                  selected ? 'text-foreground' : 'text-foreground/85',
                )}
              >
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="min-w-0 flex-1 truncate">{o.label}</span>
                    {o.badge && (
                      <span
                        className="shrink-0 rounded-full border border-mira-blue/30 bg-mira-blue/10 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wider text-mira-blue/90"
                        title="Sign-in available"
                      >
                        {o.badge}
                      </span>
                    )}
                  </div>
                  {o.hint && (
                    <div className="mt-0.5 truncate text-[11px] text-muted-foreground/80">
                      {o.hint}
                    </div>
                  )}
                </div>
                {selected && (
                  <Check className="mt-0.5 size-3.5 shrink-0 text-mira-blue" />
                )}
              </button>
            );
          })
        )}
      </PopoverContent>
    </Popover>
  );
}
