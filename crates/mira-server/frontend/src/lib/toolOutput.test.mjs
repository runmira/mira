import assert from 'node:assert/strict';
import test from 'node:test';
import { ansiToSegments, foldOutputLines, parseBashOutput } from './toolOutput.mjs';

test('parses ANSI colors as safe text segments and strips non-SGR escapes', () => {
  assert.deepEqual(
    ansiToSegments('\x1b[31mred\x1b[0m \x1b]8;;https://example.com\x07link\x1b]8;;\x07'),
    [
      { text: 'red', style: { color: '#cd3131' } },
      { text: ' link', style: {} },
    ],
  );
});

test('parses bash exit metadata and excludes backend hints from copied output', () => {
  assert.deepEqual(
    parseBashOutput('exit=2\n(command timed out)\n--- output ---\nfailed\n--- hint ---\nTry another tool'),
    { output: 'failed', exitCode: 2, timedOut: true },
  );
  assert.deepEqual(parseBashOutput('plain output'), {
    output: 'plain output',
    exitCode: null,
    timedOut: false,
  });
});

test('folds outputs over the line limit while retaining their beginning and end', () => {
  const content = Array.from({ length: 60 }, (_, i) => `line ${i + 1}`).join('\n');
  const folded = foldOutputLines(content);
  assert.equal(folded.hidden, 40);
  assert.deepEqual(folded.lines.slice(0, 2), ['line 1', 'line 2']);
  assert.deepEqual(folded.lines.slice(-2), ['line 59', 'line 60']);
  assert.equal(foldOutputLines(Array.from({ length: 50 }, (_, i) => `${i}`).join('\n')).hidden, 0);
});
