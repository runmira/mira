import { FileText, Plus } from 'lucide-react';
import { cn } from '@/lib/utils';
import { TOOL_PANE_DEFS, type ToolPaneKind } from './panes/toolPanes';

/**
 * The panel's empty state: what can be opened in the right-hand column.
 *
 * Reached by clicking the collapsed rail, by the "+" in the tab strip, or
 * by closing the last tab. Keeping it as a real view rather than a blank
 * column means the panel is never a dead end — there's always something one
 * click away.
 */
export function PanelLauncher({
  onOpenPane,
  onOpenFile,
  openFiles,
  onOpenFileTab,
  className,
}: {
  onOpenPane: (kind: ToolPaneKind) => void;
  onOpenFile: () => void;
  /** File tabs currently open, so the launcher can jump back to one. */
  openFiles: { id: string; label: string }[];
  onOpenFileTab: (id: string) => void;
  className?: string;
}) {
  const kinds: ToolPaneKind[] = ['browser', 'whiteboard', 'devtools'];

  return (
    <div className={cn('h-full overflow-y-auto p-3', className)}>
      <div className="mb-3 flex items-center gap-1.5 px-0.5 text-muted-foreground/80">
        <Plus className="size-3.5" />
        <span className="text-[11.5px] font-semibold uppercase tracking-wider">
          New
        </span>
      </div>

      <div className="flex flex-col gap-1.5">
        <Row
          title="File…"
          hint="Browse the project and open a file"
          icon={<FileText className="size-3.5" />}
          onClick={onOpenFile}
        />
        {kinds.map((kind) => (
          <Row
            key={kind}
            title={TOOL_PANE_DEFS[kind].title}
            hint={TOOL_PANE_DEFS[kind].blurb}
            icon={TOOL_PANE_DEFS[kind].icon}
            onClick={() => onOpenPane(kind)}
          />
        ))}
      </div>

      {openFiles.length > 0 && (
        <>
          <div className="mb-1.5 mt-4 px-0.5 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/80">
            Open files
          </div>
          <div className="flex flex-col gap-1.5">
            {openFiles.map((f) => (
              <Row
                key={f.id}
                title={f.label}
                icon={<FileText className="size-3.5" />}
                onClick={() => onOpenFileTab(f.id)}
              />
            ))}
          </div>
        </>
      )}
    </div>
  );
}

function Row({
  title,
  hint,
  icon,
  onClick,
}: {
  title: string;
  hint?: string;
  icon: React.ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex w-full items-center gap-2.5 rounded-lg border border-border/60 px-2.5 py-2 text-left transition-colors hover:border-border hover:bg-fg/[0.03]"
    >
      <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-fg/[0.04] text-muted-foreground">
        {icon}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[12.5px] font-medium text-foreground">
          {title}
        </span>
        {hint && (
          <span className="mt-0.5 block truncate text-[11px] text-muted-foreground/80">
            {hint}
          </span>
        )}
      </span>
    </button>
  );
}

