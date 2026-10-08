import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const { outputText } = ts.transpileModule(readFileSync(new URL('./refs.ts', import.meta.url), 'utf8'), { compilerOptions: { module: ts.ModuleKind.ESNext } });
const { parseWorkspaceFileLink } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
test('loopback workspace links preserve absolute paths and line numbers', () => {
  assert.deepEqual(parseWorkspaceFileLink('http://127.0.0.1:8797/Users/damilola/Desktop/coding_agent/mira/.mira/worktrees/main/docs/desktop.md:3'), { path:'/Users/damilola/Desktop/coding_agent/mira/.mira/worktrees/main/docs/desktop.md', line:3 });
  assert.deepEqual(parseWorkspaceFileLink('http://localhost:5174/Users/me/My%20Project/file.ts#L42'), { path:'/Users/me/My Project/file.ts', line:42 });
  assert.deepEqual(parseWorkspaceFileLink('src/main.rs:9'), { path:'src/main.rs', line:9 });
});
test('remote URLs and ordinary localhost routes stay web links', () => {
  for (const href of ['https://example.com/Users/me/file.md', 'http://localhost:8797/docs/desktop.md', 'http://127.0.0.1:8797/api/file.md', 'http://localhost:8797/Users/me/file.md?download=1', 'javascript:alert(1)']) assert.equal(parseWorkspaceFileLink(href), null);
});
