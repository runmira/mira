/** Resolve from the actual scroll offset, not a potentially stale virtual range. */
export function transcriptAnchor(keys: string[], offsets: number[], top: number) {
  if (!keys.length || !Number.isFinite(top)) return undefined;
  let low = 0, high = keys.length - 1, index = 0;
  while (low <= high) {
    const middle = (low + high) >>> 1;
    if (offsets[middle] <= top) { index = middle; low = middle + 1; }
    else high = middle - 1;
  }
  return { key: keys[index], offset: Math.max(0, top - offsets[index]), bottom: false };
}

export function anchorTop(anchor: { key: string; offset: number }, keys: string[], offsets: number[]) {
  const index = keys.indexOf(anchor.key);
  return index < 0 ? undefined : offsets[index] + Math.min(anchor.offset, Math.max(0, offsets[index + 1] - offsets[index] - 1));
}
