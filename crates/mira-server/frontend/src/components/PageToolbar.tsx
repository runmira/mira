import { cn } from '@/lib/utils';

/** The main pane's toolbar row, shared by every page (chat, Settings,
 *  plugins, pull requests…) so the title sits in one place everywhere.
 *
 *  It is `h-11`: in the desktop app its centre line is the traffic lights'
 *  (`TRAFFIC_LIGHT_INSET.centerY`), and with the sidebar hidden it is
 *  exactly as tall as the pane's notch, starting after it
 *  (`--window-notch`) with its divider running on from the notch's ledge
 *  in the same colour. It drags the window. */
export const PAGE_TOOLBAR =
  'flex h-11 shrink-0 items-center gap-3 border-b border-border/60 px-4 pl-[max(1rem,var(--window-notch,0px))] [[data-notched]_&]:border-border';

export const PAGE_TITLE = 'min-w-0 flex-1 truncate text-[15px] font-semibold tracking-tight text-foreground';

export function PageToolbar({
  title,
  children,
  className,
}: {
  title: React.ReactNode;
  /** Actions on the right. */
  children?: React.ReactNode;
  className?: string;
}) {
  return (
    <div data-window-drag-region className={cn(PAGE_TOOLBAR, className)}>
      <span className={PAGE_TITLE}>{title}</span>
      {children}
    </div>
  );
}
