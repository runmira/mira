/** Alignment harness for the agent stream.
 *
 *  Renders one turn containing every row kind the transcript can show —
 *  user bubble, duration chip, assistant prose, a grouped tool run, a
 *  single tool row, a status line — so the left gutter of each can be
 *  compared by eye and by measurement. Not shipped.
 *
 *  Toggle "Measure edges" to outline each row's first-glyph column.
 */
import { createRoot } from 'react-dom/client';
import { useState } from 'react';
import { ToolGroup } from './components/ToolGroup';
import { ToolCard } from './components/ToolCard';
import { EntryView, StatusLine } from './components/transcript/EntryView';
import { WorkedForChip } from './components/transcript/TurnView';
import { STREAM_ROW_GUTTER } from './lib/streamLayout';
import { cn } from './lib/utils';
import './styles.css';

const call = (i: number | string, name: string, args: Record<string, unknown>) => ({
  id: `align-${i}`,
  type: 'function' as const,
  function: { name, arguments: JSON.stringify(args) },
});
const grouped = ['read', 'grep', 'read'].map((tool, i) => ({
  call: call(`g${i}`, tool === 'read' ? 'read' : 'grep', tool === 'read' ? { path: `src/file${i}.ts` } : { pattern: 'useEffect' }),
  preview: null,
  status: 'complete' as const,
  result: { call_id: `g${i}`, content: '' },
}));

// A run of edits, so the group's `+N -N` has something to sum.
const edits = ['a', 'b'].map((name, i) => ({
  call: call(`e${i}`, 'edit_file', { path: `src/${name}.ts` }),
  preview: {
    path: `src/${name}.ts`,
    kind: 'edit' as const,
    lines: [
      ...Array.from({ length: 3 + i }, (_, n) => ({ tag: 'add' as const, text: `added ${n}` })),
      ...Array.from({ length: 2 + i }, (_, n) => ({ tag: 'del' as const, text: `removed ${n}` })),
    ],
  },
  status: 'complete' as const,
  result: { call_id: `e${i}`, content: '' },
}));

/** Two guides, because they are 8px apart and the gap between them is
 *  the bug this harness exists to catch: the faint line is where the
 *  column's own padding ends, the bright one is where a row actually
 *  starts. Full-bleed elements (the worked-for rule, engine-switch rules)
 *  land on the faint line instead of the bright one. */
function Measure({ on }: { on: boolean }) {
  if (!on) return null;
  return (
    <>
      <div className="pointer-events-none absolute inset-y-0 left-8 w-px bg-red-500/25" aria-hidden />
      <div className="pointer-events-none absolute inset-y-0 left-10 w-px bg-red-500/80" aria-hidden />
    </>
  );
}

function App() {
  const [measure, setMeasure] = useState(false);
  const noop = () => {};
  // The real EntryView, so what we measure is what the transcript renders.
  const msg = (text: string) => (
    <EntryView
      entry={{ kind: 'msg', msg: { role: 'assistant', content: text, created_at: 0 } }}
      onDecide={noop}
      onPlanReply={noop}
      onOpenAgent={noop}
      onAskUserReply={noop}
    />
  );
  return (
    <main className="relative mx-auto max-w-3xl px-8 py-16">
      <Measure on={measure} />
      <div className="mb-6 flex items-center gap-3">
        <button className="rounded-md border border-border px-2 py-1 text-xs" onClick={() => setMeasure((m) => !m)}>
          {measure ? 'Hide edges' : 'Measure edges'}
        </button>
        <span className="text-xs text-muted-foreground">
          Faint line = column edge. Bright line = where a row starts. Every
          row and every rule should sit on the bright one.
        </span>
      </div>

      <div className="flex flex-col gap-2">
        {/* Duration chip. The wrapper + nested rule mirrors TurnView
            exactly: the gutter goes on a padded WRAPPER, because a
            `border-b` is drawn on its own border box and can never be
            inset by padding applied to the same element. Keep these two
            lines in sync with TurnView or this harness measures itself
            instead of the app. */}
        <div className={cn('w-full', STREAM_ROW_GUTTER)}>
          <WorkedForChip
            durationMs={39000}
            active={false}
            waitingForUser={false}
            expanded
            locked={false}
            onToggle={() => {}}
            activity="3 reads, 4 searches"
          />
          <div aria-hidden className="mt-2 border-b border-border/70" />
        </div>

        {/* assistant prose */}
        {msg('Now let me trace how `devices` is wired into the server and any polling.')}

        {/* intermediate narration between tool calls — lives inside
            .transcript-work, which used to dim it to muted-foreground */}
        <div className="transcript-work">
          {msg('Wait — my test helper takes names but never uses them. Let me clean that up first.')}
        </div>

        {/* grouped tool run */}
        <ToolGroup entries={grouped} />

        {/* grouped edits — should show a summed +N -N on the right rail */}
        <ToolGroup entries={edits} />

        {/* single tool row */}
        <ToolCard
          call={call('s1', 'bash', { command: 'xcrun simctl list devices booted' })}
          preview={null}
          status="complete"
          result={{ call_id: 's1', content: 'iPhone 17 Pro (Booted)' }}
          onDecide={noop}
        />

        {/* status line (warnings / errors) */}
        <StatusLine text="Turn ended manually by user." />

        {/* trailing assistant prose */}
        {msg('The booted device came from `devices.rs`; polling is absent.')}
      </div>
    </main>
  );
}

createRoot(document.getElementById('root')!).render(<App />);