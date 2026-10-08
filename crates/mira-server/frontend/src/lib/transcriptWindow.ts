/** Measured prefix offsets allow jumps and a bounded visible window. */
export function transcriptWindow(offsets: number[], top: number, height: number, overscan = 900): { start: number; end: number } {
  const count = offsets.length - 1;
  let low = 0, high = count;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (offsets[middle + 1] < top - overscan) low = middle + 1;
    else high = middle;
  }
  const start = Math.min(low, Math.max(0, count - 1));
  let end = start;
  while (end < count && offsets[end] < top + height + overscan) end++;
  return { start, end };
}
export function quoteReply(text: string, turn: string): string {
  const number = Number(turn.replace('turn-', ''));
  const source = turn && Number.isFinite(number) ? `reply ${number + 1}` : 'your reply';
  return `Regarding ${source}:\n${text.trim().split('\n').map(line => `> ${line}`).join('\n')}\n\n`;
}
