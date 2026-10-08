/**
 * Lazily loaded diff renderers for the transcript (issue #72).
 *
 * `@pierre/diffs` and the Shiki highlighter behind it are the heaviest thing
 * the chat view used to pull into the main chunk. Tool cards and turn diffs
 * import from here instead, so the renderer loads with the first diff on
 * screen; until then the change shows as plain +/- text at the same size.
 */
import type { ComponentProps } from 'react';
import { LazyBoundary } from '../LazyBoundary';
import type { DiffLine } from '../../types';
import { lazyNamed } from '@/lib/lazy';
import { cn } from '@/lib/utils';

const InlineDiffImpl = lazyNamed(() => import('./InlineDiff'), 'InlineDiff');
const TurnSnapshotDiffImpl = lazyNamed(() => import('./TurnSnapshotDiff'), 'TurnSnapshotDiff');

const SIGN: Record<string, string> = { add: '+', del: '-', ctx: ' ' };

function PlainDiff({ lines, maxHeight = '50vh', className }: { lines: DiffLine[]; maxHeight?: string; className?: string }) {
  return (
    <pre
      className={cn('overflow-auto px-3 py-2 font-mono text-[12px] leading-[1.6] text-muted-foreground', className)}
      style={{ maxHeight }}
    >
      {lines.map((l, i) =>
        l.tag === 'hunkgap' ? '⋯\n' : <span key={i}>{`${SIGN[l.tag]} ${l.text}\n`}</span>,
      )}
    </pre>
  );
}

export function InlineDiff(props: ComponentProps<typeof InlineDiffImpl>) {
  const plain = <PlainDiff lines={props.lines} maxHeight={props.maxHeight} className={props.className} />;
  return (
    // If the renderer can't load, the plain diff stays.
    <LazyBoundary fallback={plain} errorFallback={plain}>
      <InlineDiffImpl {...props} />
    </LazyBoundary>
  );
}

export function TurnSnapshotDiff(props: ComponentProps<typeof TurnSnapshotDiffImpl>) {
  return (
    <LazyBoundary fallback={<p role="status" className="px-4 py-2 text-xs text-muted-foreground">Loading turn diff…</p>}>
      <TurnSnapshotDiffImpl {...props} />
    </LazyBoundary>
  );
}
