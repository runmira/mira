import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { transcriptAnchor, anchorTop } = await bundle(`export { transcriptAnchor, anchorTop } from './src/lib/transcriptAnchor';`);
test('anchor uses the actual fling offset, rather than a cached visible range', () => {
  const keys = Array.from({ length: 100 }, (_, i) => `row-${i}`);
  const offsets = Array.from({ length: 101 }, (_, i) => i * 100);
  const anchor = transcriptAnchor(keys, offsets, 7350);
  assert.deepEqual(anchor, { key: 'row-73', offset: 50, bottom: false });
  assert.equal(anchorTop(anchor, keys, offsets), 7350);
});
test('prepending history and remeasuring rows above preserve the reading row and offset', () => {
  const anchor = transcriptAnchor(['a','b','c'], [0,100,200,300], 150);
  assert.equal(anchorTop(anchor, ['older','a','b','c'], [0,400,650,750,850]), 700);
  assert.equal(anchorTop(anchor, ['a','b','c'], [0,250,350,450]), 300);
});
test('collapsing the reading row clamps the offset and missing rows never choose a random replacement', () => {
  const anchor = { key: 'b', offset: 200 };
  assert.equal(anchorTop(anchor, ['a','b'], [0,100,150]), 149);
  assert.equal(anchorTop(anchor, ['a'], [0,100]), undefined);
  assert.equal(transcriptAnchor([], [0], 0), undefined);
  assert.equal(transcriptAnchor(['a'], [0,100], NaN), undefined);
});
