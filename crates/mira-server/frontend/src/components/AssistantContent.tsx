import { useState } from 'react';
import { CaretDown } from '@phosphor-icons/react';
import { Markdown } from './Markdown';
import { cn } from '@/lib/utils';

/**
 * Renders assistant text, teasing apart `<think>…</think>` blocks that
 * reasoning models (DeepSeek R1, o1, Qwen QWQ, etc.) emit inline. Think
 * blocks collapse behind a "Reasoning" chevron so the transcript stays
 * scannable; the rest renders as normal markdown.
 *
 * Handles streaming — an unclosed `<think>…` while the reply is still
 * arriving renders as an open "Thinking…" block that morphs into the
 * final collapsed section once the closing tag lands.
 */
export function AssistantContent({ text }: { text: string }) {
  const segments = parseSegments(text);
  return (
    <>
      {segments.map((s, i) =>
        s.kind === 'think' ? (
          <ReasoningBlock key={i} content={s.content} open={s.streaming} />
        ) : (
          <Markdown key={i} text={s.content} />
        ),
      )}
    </>
  );
}

type Segment =
  | { kind: 'text'; content: string }
  | { kind: 'think'; content: string; streaming: boolean };

const OPEN = /<think(?:ing)?>/i;
const CLOSE = /<\/think(?:ing)?>/i;

function parseSegments(raw: string): Segment[] {
  const out: Segment[] = [];
  let rest = raw;

  while (rest.length > 0) {
    const openMatch = OPEN.exec(rest);
    if (!openMatch) {
      if (rest.trim()) out.push({ kind: 'text', content: rest });
      break;
    }

    // Text before the <think> tag.
    if (openMatch.index > 0) {
      const before = rest.slice(0, openMatch.index);
      if (before.trim()) out.push({ kind: 'text', content: before });
    }

    const afterOpen = rest.slice(openMatch.index + openMatch[0].length);
    const closeMatch = CLOSE.exec(afterOpen);

    if (closeMatch) {
      const content = afterOpen.slice(0, closeMatch.index);
      out.push({ kind: 'think', content: content.trim(), streaming: false });
      rest = afterOpen.slice(closeMatch.index + closeMatch[0].length);
    } else {
      // Streaming — closing tag hasn't arrived yet. Consume the rest as
      // an in-progress think block.
      out.push({ kind: 'think', content: afterOpen.trim(), streaming: true });
      break;
    }
  }

  return out;
}

function ReasoningBlock({ content, open: initiallyOpen }: { content: string; open: boolean }) {
  const [open, setOpen] = useState(initiallyOpen);
  const lineCount = content ? content.split('\n').length : 0;
  return (
    <div
      className={cn(
        'my-2 border-l-2 pl-2',
        initiallyOpen ? 'border-mira-purple' : 'border-mira-purple/35',
      )}
    >
      <button
        onClick={() => setOpen((v) => !v)}
        className="inline-flex items-center gap-2 rounded px-2 py-1 text-[12.5px] text-muted-foreground transition-colors hover:bg-accent/40 hover:text-foreground"
      >
        <CaretDown
          weight="bold"
          className={cn(
            'size-3 text-muted-foreground/60 transition-transform',
            !open && '-rotate-90',
            open && 'text-muted-foreground',
          )}
        />
        <span
          className={cn(
            'font-medium text-mira-purple',
            initiallyOpen && 'animate-pulse',
          )}
        >
          {initiallyOpen ? 'Reasoning…' : 'Reasoning'}
        </span>
        {!initiallyOpen && lineCount > 0 && (
          <span className="font-mono text-[11px] text-muted-foreground/70">
            {lineCount} line{lineCount === 1 ? '' : 's'}
          </span>
        )}
      </button>
      {open && content && (
        <div className="mt-1.5 ml-2 mb-2 animate-fade-in rounded-md border border-mira-purple/15 bg-mira-purple/[0.04] px-3 py-2 text-[13.5px] text-muted-foreground">
          <Markdown text={content} />
        </div>
      )}
    </div>
  );
}
