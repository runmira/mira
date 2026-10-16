/** The text a set of notes becomes when the message is sent. Their
 *  screenshots go with it, in the same order. */
export function notesMarkdown(notes: { url: string; target: string | null; comment: string }[]): string {
  if (notes.length === 0) return '';
  const pages = [...new Set(notes.map((n) => n.url).filter(Boolean))];
  const head = pages.length === 1 ? `Notes on ${pages[0]}` : 'Notes on the browser page';
  const lines = notes.map((n, i) => {
    const where = n.target ?? 'the marked spot';
    const page = pages.length > 1 && n.url ? ` (${n.url})` : '';
    return `${i + 1}. At ${where}${page}: ${n.comment}`;
  });
  const shots = notes.length === 1
    ? 'The attached screenshot marks it.'
    : 'The attached screenshots mark each one, in order.';
  return `${head}:\n${lines.join('\n')}\n${shots}\n\n`;
}
