/** The current nonempty line; its start offset stays stable as tokens append.
 * Behavior adapted from ZCode's reasoning summary (Apache-2.0). */
export function thinkingPreview(content: string) {
  const text = content.replace(/\r\n?/g, '\n').trimEnd();
  const start = text.lastIndexOf('\n') + 1;
  return { key: String(start), text: text.slice(start).trim() };
}

export function thinkingScrollEdges(el: { scrollTop: number; scrollHeight: number; clientHeight: number }) {
  return { top: el.scrollTop > 1, bottom: el.scrollHeight - el.clientHeight - el.scrollTop > 1 };
}
