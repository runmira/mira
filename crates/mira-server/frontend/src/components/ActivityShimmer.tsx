import type { ReactNode } from 'react';
import { cn } from '../lib/utils';
/** Theme tokens keep the moving band readable on both light and dark surfaces. */
export function ActivityShimmer({ active, children, className }: { active: boolean; children: ReactNode; className?: string }) {
  return <span className={cn('text-muted-foreground', active && 'activity-shimmer animate-text-shimmer', className)}>{children}</span>;
}
