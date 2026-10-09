import { useFileIcons } from '@/lib/fileIcons';
import { cn } from '@/lib/utils';
import { ChevronDown, ChevronRight, Folder } from 'lucide-react';
/** A collapsible shelf's heading: Recents, Settled, Archived. */
export function ShelfHeader({
  icon,
  label,
  count,
  open,
  onToggle,
}: {
  icon: React.ReactNode;
  label: string;
  count?: number;
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={open}
      className="flex items-center gap-1.5 rounded-md px-2 py-1 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/80 transition-colors hover:bg-fg/[0.05] hover:text-foreground"
    >
      {open ? (
        <ChevronDown className="size-3 shrink-0 text-muted-foreground/60" />
      ) : (
        <ChevronRight className="size-3 shrink-0 text-muted-foreground/60" />
      )}
      {icon}
      {label}
      {count != null && count > 0 && (
        <span className="rounded-full bg-secondary/80 px-1.5 text-[10.5px] font-medium normal-case tracking-normal text-muted-foreground">
          {count}
        </span>
      )}
    </button>
  );
}

/** Themed folder icon for a project row, falling back to the lucide glyph
 *  while the icon set loads. */
export function SidebarFolderIcon({
  name,
  open,
  className,
  current,
}: {
  name: string;
  open: boolean;
  className?: string;
  current?: boolean;
}) {
  const { folderIcon } = useFileIcons();
  const dataUri = folderIcon(name, open);
  if (dataUri) {
    return (
      <img
        src={dataUri}
        alt=""
        className={cn('shrink-0', className, !current && 'opacity-80')}
        draggable={false}
      />
    );
  }
  return (
    <Folder
      className={cn('shrink-0', className, current ? 'text-mira-blue' : 'text-muted-foreground/70')}
    />
  );
}

/* ---------- little helpers ---------- */

export function NavItem({
  icon,
  disabled,
  active,
  onClick,
  onIntent,
  children,
}: {
  icon: React.ReactNode;
  disabled?: boolean;
  /** True when this item's view is currently rendered in the main pane —
   *  gets the same accent treatment as an active session row. */
  active?: boolean;
  onClick?: () => void;
  /** Hover/focus: preload whatever the click opens. */
  onIntent?: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      disabled={disabled}
      onClick={onClick}
      onPointerEnter={onIntent}
      onFocus={onIntent}
      title={disabled ? 'Not implemented yet' : undefined}
      className={cn(
        'flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left text-[14.5px] transition-colors',
        disabled
          ? 'text-muted-foreground/40 cursor-not-allowed'
          : active
            ? 'sidebar-active bg-fg/[0.1] text-foreground'
            : 'text-foreground hover:bg-fg/[0.05]',
      )}
    >
      <span
        className={cn(
          'shrink-0',
          disabled
            ? 'text-muted-foreground/40'
            : active
              ? 'text-foreground'
              : 'text-muted-foreground',
        )}
      >
        {icon}
      </span>
      <span>{children}</span>
    </button>
  );
}

export function Empty({ children }: { children: React.ReactNode }) {
  return <div className="px-2.5 py-1 text-[13px] text-muted-foreground/60">{children}</div>;
}
