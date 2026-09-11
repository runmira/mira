import { useState } from 'react';
import { CaretDown, CheckCircle, Circle, CircleNotch } from '@phosphor-icons/react';
import { cn } from '@/lib/utils';
import type { TaskItem } from '../types';

type Props = {
  tasks: TaskItem[];
};

/**
 * Persistent "current plan" card that surfaces the model's task list at a
 * glance. Rendered when at least one non-deleted task exists.
 *
 * Data comes from the `data` payload on `task_*` tool results (upserted in
 * App.tsx) and the initial hydration from `ready.tasks` on socket open, so
 * the panel survives reloads.
 */
export function TaskListPanel({ tasks }: Props) {
  const [open, setOpen] = useState(true);
  const visible = tasks.filter((t) => t.status !== 'deleted');
  if (visible.length === 0) return null;

  const completed = visible.filter((t) => t.status === 'completed').length;
  const inProgress = visible.find((t) => t.status === 'in_progress');
  const total = visible.length;
  // Sort so in_progress bubbles above pending, completed sinks to the
  // bottom — same shape a human uses when scanning a todo list.
  const sorted = [...visible].sort(
    (a, b) => statusRank(a.status) - statusRank(b.status) || a.id - b.id,
  );

  return (
    <div className="flex justify-start">
      <div className="w-full max-w-2xl rounded-lg border border-mira-border/50 bg-mira-panel/60 shadow-sm">
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          className="flex w-full items-center gap-2 rounded-t-lg px-3 py-2 text-left transition hover:bg-mira-panel"
          aria-expanded={open}
        >
          <CaretDown
            weight="bold"
            className={cn(
              'size-3 shrink-0 text-muted-foreground/60 transition-transform',
              !open && '-rotate-90',
              open && 'text-muted-foreground',
            )}
          />
          <span className="font-medium text-[13px] text-mira-fg">Plan</span>
          <span className="text-[12px] text-mira-muted">
            {completed}/{total} complete
            {inProgress ? ` · ${activeLabel(inProgress)}` : ''}
          </span>
        </button>
        {open && (
          <ul className="border-t border-mira-border/40 px-3 py-2">
            {sorted.map((t) => (
              <li key={t.id} className="flex items-start gap-2 py-1 text-[13px]">
                <StatusIcon status={t.status} />
                <span
                  className={
                    t.status === 'completed'
                      ? 'min-w-0 break-words text-mira-muted line-through'
                      : t.status === 'in_progress'
                        ? 'min-w-0 break-words text-mira-fg font-medium'
                        : 'min-w-0 break-words text-mira-fg/85'
                  }
                >
                  {labelFor(t)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function StatusIcon({ status }: { status: TaskItem['status'] }) {
  switch (status) {
    case 'completed':
      return <CheckCircle size={16} weight="fill" className="mt-[2px] shrink-0 text-emerald-400" />;
    case 'in_progress':
      return <CircleNotch size={16} weight="bold" className="mt-[2px] shrink-0 animate-spin text-mira-blue" />;
    default:
      return <Circle size={16} className="mt-[2px] shrink-0 text-mira-muted" />;
  }
}

function statusRank(status: TaskItem['status']): number {
  // Lower ranks render first.
  switch (status) {
    case 'in_progress':
      return 0;
    case 'pending':
      return 1;
    case 'completed':
      return 2;
    default:
      return 3;
  }
}

function labelFor(t: TaskItem): string {
  if (t.status === 'in_progress' && t.active_form) return t.active_form;
  return t.subject;
}

function activeLabel(t: TaskItem): string {
  return t.active_form || t.subject;
}
