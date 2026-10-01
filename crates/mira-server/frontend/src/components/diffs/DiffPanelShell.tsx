/**
 * Drawer shell + loading skeletons for the diff panel.
 */
import type { ReactNode } from 'react';

import { cn } from '@/lib/utils';

export type DiffPanelMode = 'inline' | 'sheet' | 'sidebar' | 'embedded';

function getDiffPanelHeaderRowClassName(mode: DiffPanelMode) {
  return cn(
    'flex h-10 min-h-10 shrink-0 items-center justify-between gap-2 border-b border-border/60 bg-background',
    mode === 'embedded' && 'px-2',
    mode !== 'embedded' && 'px-4',
  );
}

export function DiffPanelShell(props: {
  mode: DiffPanelMode;
  header: ReactNode;
  children: ReactNode;
}) {
  return (
    <div
      className={cn(
        'flex h-full min-w-0 flex-col bg-background',
        props.mode === 'inline'
          ? 'w-[42vw] min-w-[360px] max-w-[560px] shrink-0 border-l border-border'
          : 'w-full',
      )}
    >
      <div className={getDiffPanelHeaderRowClassName(props.mode)} data-surface-subheader>
        {props.header}
      </div>
      {props.children}
    </div>
  );
}

export function DiffFileHeaderSkeleton({
  titleWidth,
}: {
  titleWidth: 'short' | 'medium' | 'long';
}) {
  return (
    <div className="flex h-8 items-center gap-2 px-2 pr-3">
      <div className="flex size-5 shrink-0 items-center justify-center">
        <div className="size-2.5 animate-pulse rounded-full bg-secondary" />
      </div>
      <div className="size-5 shrink-0 animate-pulse rounded bg-secondary" />
      <div
        className={
          titleWidth === 'short'
            ? 'h-3 w-2/5 max-w-52 animate-pulse rounded-full bg-secondary'
            : titleWidth === 'medium'
              ? 'h-3 w-1/2 max-w-64 animate-pulse rounded-full bg-secondary'
              : 'h-3 w-3/5 max-w-72 animate-pulse rounded-full bg-secondary'
        }
      />
      <div className="ml-auto flex shrink-0 items-center gap-2">
        <div className="h-3 w-5 animate-pulse rounded-full bg-secondary" />
        <div className="h-3 w-5 animate-pulse rounded-full bg-secondary" />
      </div>
    </div>
  );
}

function DiffCodeLineSkeleton({ width }: { width: 'short' | 'medium' | 'long' }) {
  return (
    <div className="flex items-center gap-3">
      <div className="h-2.5 w-5 shrink-0 animate-pulse rounded-full bg-secondary" />
      <div
        className={
          width === 'short'
            ? 'h-2.5 w-3/5 animate-pulse rounded-full bg-secondary'
            : width === 'medium'
              ? 'h-2.5 w-2/3 animate-pulse rounded-full bg-secondary'
              : 'h-2.5 w-4/5 animate-pulse rounded-full bg-secondary'
        }
      />
    </div>
  );
}

export function DiffPanelLoadingState(props: { label: string }) {
  return (
    <div
      className="min-h-0 flex-1 overflow-hidden bg-background"
      role="status"
      aria-live="polite"
      aria-label={props.label}
    >
      <DiffFileHeaderSkeleton titleWidth="medium" />
      <div className="flex h-6 items-center gap-2 px-2 pr-3">
        <div className="h-px flex-1 bg-border/40" />
        <div className="h-2.5 w-24 animate-pulse rounded-full bg-secondary" />
        <div className="h-px flex-1 bg-border/40" />
      </div>
      <div className="space-y-2 px-3 py-2">
        <DiffCodeLineSkeleton width="medium" />
        <DiffCodeLineSkeleton width="long" />
        <DiffCodeLineSkeleton width="short" />
      </div>
      <DiffFileHeaderSkeleton titleWidth="short" />
      <DiffFileHeaderSkeleton titleWidth="long" />
      <span className="sr-only">{props.label}</span>
    </div>
  );
}
