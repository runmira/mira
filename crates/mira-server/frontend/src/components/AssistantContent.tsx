import { Markdown } from './Markdown';
import { ThoughtBlock } from './ThoughtBlock';
import type { DiffPreview } from '../types';

/**
 * Renders assistant text, teasing apart `<think>…</think>` blocks that
 * reasoning models (DeepSeek R1, o1, Qwen QWQ, etc.) emit inline. Think
 * blocks collapse behind a "Thought" chevron so the transcript stays
 * scannable; the rest renders as normal markdown.
 *
 * Handles streaming — an unclosed `<think>…` while the reply is still
 * arriving renders as an open "Thinking…" block that morphs into the
 * final collapsed section once the closing tag lands.
 */
export function AssistantContent({ text, onOpenFile }: { text: string; onOpenFile?: (path: string, diff: DiffPreview | null) => void }) {
  const segments = parseSegments(text);
  return (
    <>
      {segments.map((s, i) =>
        s.kind === 'think' ? (
          <ThoughtBlock key={i} content={s.content} live={s.streaming} />
        ) : (
          <Markdown key={i} text={s.content} onOpenFile={onOpenFile} />
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
