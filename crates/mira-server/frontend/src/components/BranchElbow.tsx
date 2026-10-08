import { cn } from '../lib/utils';

/** The same rounded branch connector used for sidebar forks. */
export function BranchElbow({ className }: { className?: string }) {
  return <span aria-hidden="true" className={cn('pointer-events-none absolute h-4 w-2.5 rounded-bl-[7px] border-b border-l border-fg/15', className)} />;
}
