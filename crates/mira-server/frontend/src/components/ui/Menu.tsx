import * as React from 'react';
import * as PopoverPrimitive from '@radix-ui/react-popover';
import { cn } from '@/lib/utils';

/**
 * Menu primitive.
 *
 * Radix ships `Popover` but not a menu with listbox semantics, and this app
 * has no `dropdown-menu` dependency. Rather than hand-roll positioning and
 * dismissal twice, the shell here is a Popover (which already handles
 * outside-click, Escape, focus return, and portal/collision) and this file
 * adds what a menu needs on top: roving focus with arrow keys, Home/End,
 * typeahead, and a disabled reason per item.
 *
 * Visual spec, applied to every menu in the app:
 *   - `rounded-lg` shell, `rounded-md` items, 2px between items
 *   - a single 10%-white hairline; no stacked ring or translucent border
 *   - overlay-level shadow only — menus sit on the content surface, so they
 *     don't need the attention-level shadow toasts get
 *   - rows are two-line capable: `label` plus an optional muted `hint`
 */

type MenuContextValue = {
  contentRef: React.RefObject<HTMLDivElement | null>;
  close: () => void;
};

const MenuContext = React.createContext<MenuContextValue | null>(null);

export function Menu({
  open,
  onOpenChange,
  align = 'end',
  side = 'bottom',
  sideOffset = 6,
  className,
  children,
  /** Accessible label for the menu surface. */
  label,
  minWidth,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  align?: 'start' | 'center' | 'end';
  side?: 'top' | 'bottom';
  sideOffset?: number;
  className?: string;
  children: React.ReactNode;
  label: string;
  minWidth?: number;
}) {
  // The caller wraps its trigger in a `relative` container; this
  // zero-height overlay is what Popover measures, so the menu lines up with
  // the trigger without the caller having to forward a ref through
  // `Anchor asChild`.
  const contentRef = React.useRef<HTMLDivElement | null>(null);
  const typeahead = React.useRef({ query: '', at: 0 });
  const [typeaheadTick, setTypeaheadTick] = React.useState(0);

  const ctx = React.useMemo<MenuContextValue>(
    () => ({ contentRef, close: () => onOpenChange(false) }),
    [onOpenChange],
  );

  function items(): HTMLElement[] {
    if (!contentRef.current) return [];
    return Array.from(
      contentRef.current.querySelectorAll<HTMLElement>('[data-menu-item]:not([disabled])'),
    );
  }

  function focusAt(i: number) {
    const list = items();
    if (list.length === 0) return;
    const idx = (i + list.length) % list.length;
    list[idx].focus();
  }

  function onKeyDown(e: React.KeyboardEvent) {
    const list = items();
    if (list.length === 0) return;
    const current = list.indexOf(document.activeElement as HTMLElement);

    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        focusAt(current + 1);
        return;
      case 'ArrowUp':
        e.preventDefault();
        // From nothing focused, ArrowUp lands on the last item.
        focusAt(current <= 0 ? list.length - 1 : current - 1);
        return;
      case 'Home':
        e.preventDefault();
        focusAt(0);
        return;
      case 'End':
        e.preventDefault();
        focusAt(list.length - 1);
        return;
      case 'Tab':
        // A menu is a dead end for tabbing; Escape or an explicit choice
        // is the way out.
        e.preventDefault();
        return;
    }

    // Typeahead: printable characters jump to the next item starting with
    // that prefix. The 600ms window matches the usual menu convention.
    if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
      const now = Date.now();
      const t = typeahead.current;
      t.query = now - t.at > 600 ? e.key : t.query + e.key;
      t.at = now;
      const needle = t.query.toLowerCase();
      // setTypeaheadTick forces a re-render so the highlighted item shows.
      setTypeaheadTick((n) => n + 1);
      const from = current + 1;
      for (let i = 0; i < list.length; i++) {
        const el = list[(from + i) % list.length];
        const label = el.dataset.menuItem ?? '';
        if (label.toLowerCase().startsWith(needle)) {
          el.focus();
          break;
        }
      }
    }
    void typeaheadTick;
  }

  return (
    <MenuContext.Provider value={ctx}>
      <PopoverPrimitive.Root open={open} onOpenChange={onOpenChange}>
        <PopoverPrimitive.Anchor className="pointer-events-none absolute inset-x-0 top-0 h-full" />
        <PopoverPrimitive.Portal>
          <PopoverPrimitive.Content
            ref={contentRef}
            align={align}
            side={side}
            sideOffset={sideOffset}
            aria-label={label}
            role="menu"
            style={minWidth ? { minWidth } : undefined}
            onKeyDown={onKeyDown}
            className={cn(
              'z-50 rounded-lg border border-border bg-popover p-1 text-popover-foreground outline-none',
              'shadow-[0_12px_32px_-8px_rgba(0,0,0,0.7)]',
              'data-[state=open]:animate-fade-in',
              className,
            )}
          >
            <div className="flex flex-col gap-0.5">{children}</div>
          </PopoverPrimitive.Content>
        </PopoverPrimitive.Portal>
      </PopoverPrimitive.Root>
    </MenuContext.Provider>
  );
}

export function MenuLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="px-2 pb-1 pt-1.5 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground/70">
      {children}
    </div>
  );
}

export function MenuSeparator() {
  return <div className="my-1 h-px bg-border" />;
}

export function MenuItem({
  label,
  hint,
  icon,
  active,
  disabled,
  disabledReason,
  onSelect,
  className,
}: {
  /** Doubles as the typeahead key and the a11y name. */
  label: string;
  /** Muted second line. Omit for a single-line row. */
  hint?: string;
  icon?: React.ReactNode;
  /** Renders a trailing state marker (e.g. the open pane). */
  active?: boolean;
  disabled?: boolean;
  /** Why it's disabled. Announced and shown on hover. */
  disabledReason?: string;
  onSelect: () => void;
  className?: string;
}) {
  const ctx = React.useContext(MenuContext);
  const ref = React.useRef<HTMLButtonElement | null>(null);

  function activate() {
    if (disabled) return;
    onSelect();
    ctx?.close();
  }

  return (
    <button
      ref={ref}
      type="button"
      role="menuitem"
      data-menu-item={label}
      disabled={disabled}
      title={disabled ? disabledReason : undefined}
      onClick={activate}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          activate();
        }
      }}
      className={cn(
        'flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors',
        'focus:outline-none focus-visible:bg-white/[0.1]',
        disabled
          ? 'cursor-not-allowed text-muted-foreground/40'
          : 'text-foreground hover:bg-white/[0.05]',
        className,
      )}
    >
      {icon && (
        <span className="flex size-4 shrink-0 items-center justify-center text-muted-foreground">
          {icon}
        </span>
      )}
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[12.5px] leading-tight font-medium">
          {label}
        </span>
        {hint && (
          <span className="mt-0.5 block truncate text-[11px] leading-tight text-muted-foreground/75">
            {disabled && disabledReason ? disabledReason : hint}
          </span>
        )}
      </span>
      {active && (
        <span
          className="size-1.5 shrink-0 rounded-full bg-foreground/70"
          aria-label="currently open"
        />
      )}
    </button>
  );
}
