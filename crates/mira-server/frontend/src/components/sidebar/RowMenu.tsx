import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';
import {
  Archive,
  ArchiveRestore,
  CheckCheck,
  ChevronRight,
  CircleCheck,
  Ellipsis,
  Pencil,
  Pin,
  PinOff,
  Timer,
  Trash2,
} from 'lucide-react';
import { useRef, useState } from 'react';
import type { BackgroundMode, SessionSummary } from '../../types';
/* ---------- row overflow menu (extensible) ---------- */

export type RowMenuItem = {
  label: string;
  icon?: React.ReactNode;
  danger?: boolean;
  /** Confirmation prompt shown before running `onSelect`. Skips confirm if null. */
  confirm?: string | null;
  /** Optional check-mark rendered on the right — used by radio-style
   *  sub-items so the current background mode reads at a glance. */
  checked?: boolean;
  description?: string;
  children?: RowMenuItem[];
  onSelect?: () => void | Promise<void>;
};

/** RowMenu items for a session row. Renders rename + pin/archive (when
 *  wired) + delete plus, when the caller wired a background-mode handler,
 *  a submenu of three choices for the current per-slot policy. The three
 *  modes always render (rather than hiding when the slot isn't loaded) so
 *  the user can see the choice; clicking on a persisted-but-not-loaded
 *  row 404s — callers should typically attach first. */
export function backgroundMenuItems(args: {
  session: SessionSummary;
  onRename: () => void;
  onDelete: () => void;
  onSetBackgroundMode?: (mode: BackgroundMode) => void;
  onPin?: (pinned: boolean) => void;
  onArchive?: () => void;
  settled?: boolean;
  onSettle?: (settled: boolean) => void;
}): RowMenuItem[] {
  const items: RowMenuItem[] = [
    {
      label: 'Rename session',
      icon: <Pencil className="size-3.5" />,
      onSelect: args.onRename,
    },
  ];
  if (args.onPin) {
    items.push({
      label: args.session.pinned ? 'Unpin session' : 'Pin session',
      icon: args.session.pinned ? <PinOff className="size-3.5" /> : <Pin className="size-3.5" />,
      onSelect: () => args.onPin!(!args.session.pinned),
    });
  }
  if (args.onSettle) {
    items.push({
      label: args.settled ? 'Move back to project' : 'Settle',
      icon: args.settled ? (
        <ArchiveRestore className="size-3.5" />
      ) : (
        <CheckCheck className="size-3.5" />
      ),
      onSelect: () => args.onSettle!(!args.settled),
    });
  }
  if (args.onArchive) {
    items.push({
      label: 'Archive session',
      icon: <Archive className="size-3.5" />,
      onSelect: args.onArchive,
    });
  }
  if (args.onSetBackgroundMode) {
    const current = args.session.background_mode ?? null;
    const modes: Array<{ mode: BackgroundMode; label: string; desc: string }> = [
      { mode: 'deny', label: 'Auto-deny', desc: 'Deny tool approvals while you are away.' },
      {
        mode: 'auto_approve',
        label: 'Auto-approve',
        desc: 'Allow tool approvals while you are away.',
      },
      { mode: 'park', label: 'Wait for me', desc: 'Hold tool approvals until you return.' },
    ];
    items.push({
      label: 'Background mode',
      icon: <Timer className="size-3.5" />,
      children: modes.map((m) => ({
        label: m.label,
        description: m.desc,
        checked: current === m.mode,
        onSelect: () => args.onSetBackgroundMode!(m.mode),
      })),
    });
  }
  items.push({
    label: 'Delete session',
    danger: true,
    confirm: 'Delete this session? This cannot be undone.',
    icon: <Trash2 className="size-3.5" />,
    onSelect: args.onDelete,
  });
  return items;
}

export function RowSubmenu({
  item,
  onPick,
}: {
  item: RowMenuItem;
  onPick: (item: RowMenuItem) => void;
}) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          ref={trigger}
          type="button"
          onKeyDown={(e) => {
            if (e.key === 'ArrowRight') {
              e.preventDefault();
              setOpen(true);
            }
          }}
          className="flex items-center gap-2 rounded-lg px-2 py-1.5 text-left text-[13px] text-foreground transition-colors hover:bg-fg/[0.05]"
        >
          <span className="shrink-0 text-muted-foreground">{item.icon}</span>
          <span className="flex-1">{item.label}</span>
          <ChevronRight className="size-3.5 text-muted-foreground" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        side="right"
        align="start"
        sideOffset={8}
        className="sidebar-menu w-60 p-1"
        aria-label={item.label}
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === 'ArrowLeft') {
            e.preventDefault();
            e.stopPropagation();
            setOpen(false);
            trigger.current?.focus();
          }
        }}
      >
        <div className="px-2 pb-1 pt-1.5 text-[10.5px] font-medium text-muted-foreground">
          When you leave this chat
        </div>
        {item.children?.map((child) => (
          <button
            type="button"
            key={child.label}
            aria-pressed={child.checked}
            onClick={() => {
              setOpen(false);
              onPick(child);
            }}
            className="flex w-full items-start gap-2 rounded-lg px-2 py-2 text-left transition-colors hover:bg-fg/[0.05]"
          >
            <span className="min-w-0 flex-1">
              <span className="block text-[12.5px] font-medium text-foreground">{child.label}</span>
              <span className="mt-0.5 block text-[10.5px] leading-4 text-muted-foreground">
                {child.description}
              </span>
            </span>
            {child.checked && <CircleCheck className="mt-0.5 size-3.5 shrink-0 text-mira-blue" />}
          </button>
        ))}
      </PopoverContent>
    </Popover>
  );
}

export function RowMenu({ items }: { items: RowMenuItem[] }) {
  const [open, setOpen] = useState(false);
  const [confirming, setConfirming] = useState<RowMenuItem | null>(null);

  function pick(item: RowMenuItem) {
    if (item.confirm) {
      setConfirming(item);
    } else {
      setOpen(false);
      void item.onSelect?.();
    }
  }

  return (
    <Popover
      open={open}
      onOpenChange={(v) => {
        setOpen(v);
        if (!v) setConfirming(null);
      }}
    >
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label="Row menu"
          onClick={(e) => e.stopPropagation()}
          className={cn(
            'shrink-0 rounded-sm p-0.5 text-muted-foreground/60 transition-opacity hover:bg-fg/[0.05] hover:text-foreground touch:p-1.5',
            open ? 'opacity-100' : 'opacity-0 group-hover:opacity-100 focus:opacity-100',
          )}
        >
          <Ellipsis className="size-3.5" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        className="sidebar-menu w-56 p-1"
        align="end"
        // Prevent the enclosing row's onClick from firing when the user
        // clicks anywhere inside the menu popover.
        onClick={(e) => e.stopPropagation()}
      >
        {confirming ? (
          <div className="flex flex-col gap-2 px-2 py-1.5">
            <div className="text-[12.5px] text-foreground">{confirming.confirm}</div>
            <div className="flex justify-end gap-1.5">
              <button
                type="button"
                onClick={() => setConfirming(null)}
                className="rounded-md px-2 py-1 text-[12px] text-muted-foreground hover:bg-fg/[0.05]"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={async () => {
                  const item = confirming;
                  setConfirming(null);
                  setOpen(false);
                  await item.onSelect?.();
                }}
                className={cn(
                  'rounded-md px-2 py-1 text-[12px] font-medium',
                  confirming.danger
                    ? 'bg-destructive text-white hover:opacity-90'
                    : 'bg-foreground text-background hover:opacity-90',
                )}
              >
                {confirming.danger ? 'Delete' : 'OK'}
              </button>
            </div>
          </div>
        ) : (
          <div className="flex flex-col">
            {items.map((it, i) =>
              it.children ? (
                <RowSubmenu key={it.label} item={it} onPick={pick} />
              ) : (
                <button
                  key={i}
                  type="button"
                  onClick={() => pick(it)}
                  className={cn(
                    'flex items-center gap-2 rounded-lg px-2 py-1.5 text-left text-[13px] transition-colors',
                    it.danger
                      ? 'text-destructive hover:bg-destructive/10'
                      : 'text-foreground hover:bg-fg/[0.05]',
                  )}
                >
                  {it.icon && <span className="shrink-0 text-muted-foreground">{it.icon}</span>}
                  <span className="flex-1">{it.label}</span>
                  {it.checked && <CircleCheck className="size-3.5 shrink-0 text-emerald-500" />}
                </button>
              ),
            )}
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}
