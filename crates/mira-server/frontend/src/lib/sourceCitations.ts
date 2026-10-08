export type SourceCitation = { session: string; turn: string; quote: string; before: string; after: string };
export function citationHref(citation: SourceCitation): string {
  return `#mira-citation=${encodeURIComponent(JSON.stringify(citation)).replace(/[!'()*]/g, char => `%${char.charCodeAt(0).toString(16).toUpperCase()}`)}`;
}
export function parseCitation(href: string): SourceCitation | null {
  const prefix = '#mira-citation=';
  if (!href.startsWith(prefix)) return null;
  try {
    const value = JSON.parse(decodeURIComponent(href.slice(prefix.length)));
    return value && ['session', 'turn', 'quote', 'before', 'after'].every(key => typeof value[key] === 'string') && value.quote.length > 0 ? value : null;
  } catch { return null; }
}

export type ComposerCitationSegment =
  | { kind: 'text'; text: string }
  | { kind: 'citation'; label: string; raw: string; citation: SourceCitation };

/** The editor displays a short label while retaining the canonical Markdown
 * reference for submission, draft persistence and source navigation. */
export function parseComposerCitations(text: string): ComposerCitationSegment[] {
  const segments: ComposerCitationSegment[] = [];
  const links = /\[([^\]\n]+)\]\((#mira-citation=[^\s)]+)\)/g;
  let start = 0;
  for (const match of text.matchAll(links)) {
    const citation = parseCitation(match[2]);
    if (!citation) continue;
    const index = match.index!;
    if (index > start) segments.push({ kind: 'text', text: text.slice(start, index) });
    segments.push({ kind: 'citation', label: match[1], raw: match[0], citation });
    start = index + match[0].length;
  }
  if (start < text.length) segments.push({ kind: 'text', text: text.slice(start) });
  return segments;
}
/** Context disambiguates repeated quotations without depending on DOM node IDs. */
export function citationOffset(text: string, citation: SourceCitation): number {
  for (let index = text.indexOf(citation.quote); index >= 0; index = text.indexOf(citation.quote, index + 1)) {
    if (text.slice(Math.max(0, index - citation.before.length), index) === citation.before && text.slice(index + citation.quote.length, index + citation.quote.length + citation.after.length) === citation.after) return index;
  }
  return -1;
}
export function resolveCitationRange(source: HTMLElement, citation: SourceCitation): Range | null {
  const nodes: Text[] = [];
  const walker = document.createTreeWalker(source, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) nodes.push(walker.currentNode as Text);
  const offset = citationOffset(nodes.map(node => node.data).join(''), citation);
  if (offset < 0) return null;
  let consumed = 0;
  const range = document.createRange();
  let started = false;
  for (const node of nodes) {
    if (!started && consumed + node.length >= offset) { range.setStart(node, offset - consumed); started = true; }
    if (started && consumed + node.length >= offset + citation.quote.length) { range.setEnd(node, offset + citation.quote.length - consumed); return range; }
    consumed += node.length;
  }
  return null;
}
