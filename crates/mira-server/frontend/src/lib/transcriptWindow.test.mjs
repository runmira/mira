import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';
const source = await readFile(new URL('./transcriptWindow.ts', import.meta.url), 'utf8');
const output = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const { transcriptWindow, quoteReply } = await import(`data:text/javascript;base64,${Buffer.from(output).toString('base64')}`);
test('long transcripts mount only the nearby window, including the final turn', () => {
  const offsets = Array.from({ length: 10001 }, (_, index) => index * 200);
  const middle = transcriptWindow(offsets, 1000000, 600);
  assert.ok(middle.start <= 5000 && middle.end > 5000);
  assert.ok(middle.end - middle.start < 20);
  const last = transcriptWindow(offsets, 1999400, 600);
  assert.equal(last.end, 10000);
  assert.deepEqual(transcriptWindow([0], 0, 600), { start: 0, end: 0 });
});
test('quoted selections preserve line breaks and include their source reply', () => {
  assert.equal(quoteReply('first line\n\nlast line', 'turn-42'), 'Regarding reply 43:\n> first line\n> \n> last line\n\n');
  assert.ok(quoteReply('text', '').startsWith('Regarding your reply:'));
});
