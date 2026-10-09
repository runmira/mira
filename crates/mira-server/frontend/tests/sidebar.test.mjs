import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

function compiled(path) {
  return ts.transpileModule(readFileSync(new URL(path, import.meta.url), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.ESNext },
  }).outputText;
}
function moduleUrl(source) { return `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`; }
const settled = moduleUrl(compiled('../src/lib/settled.ts'));
const source = compiled('../src/lib/sidebar.ts').replace("'./settled'", JSON.stringify(settled));
const { matchesSidebar, rowStatus, settledPeriod } = await import(moduleUrl(source));
const chat = { title: 'Fix authentication', cwd: '/work/Mira', updated_at: 1000,
  worktree_branch: 'fix/auth', pr: { number: 179, title: 'Restore login', state: 'open' } };

test('search finds PR numbers, branch, project, and combined words', () => {
  for (const query of ['#179', '179', 'fix/auth', 'mira', 'restore login', 'mira auth']) {
    assert.equal(matchesSidebar(chat, query, 'all'), true, query);
  }
  assert.equal(matchesSidebar(chat, 'unknown', 'all'), false);
});

test('filters keep waiting work out of Working and failures out of Settled', () => {
  const waiting = { ...chat, running: true, needs_attention: true, attention_reason: 'Approve 3 actions' };
  assert.equal(matchesSidebar(waiting, '', 'running'), false);
  assert.equal(matchesSidebar(waiting, '', 'waiting'), true);
  const failed = { ...chat, pr: null, worktree_status: 'merged', failure_reason: 'Failed' };
  assert.equal(matchesSidebar(failed, '', 'waiting'), true);
  assert.equal(matchesSidebar(failed, '', 'settled'), false);
});

test('one primary status prioritizes attention, failure, running, and unread results', () => {
  assert.deepEqual(rowStatus({ ...chat, running: true, needs_attention: true, attention_reason: 'Approve 3 actions' }, true),
    { label: 'Approve 3 actions', tone: 'attention' });
  assert.equal(rowStatus({ ...chat, running: true }, true).label, 'Working');
  assert.equal(rowStatus({ ...chat, failure_reason: 'Error' }, true).label, 'Needs a retry');
  assert.deepEqual(rowStatus({ ...chat, running: true, failure_reason: 'Error' }),
    { label: 'Needs a retry', tone: 'error' });
  assert.equal(rowStatus(chat, true).label, 'PR ready');
  assert.equal(rowStatus({ ...chat, pr: null }, true).label, 'Finished');
  assert.equal(rowStatus(chat).label, 'PR open');
});

test('settled dates use the finish time and local calendar boundaries', () => {
  const now = new Date(2026, 9, 9, 12);
  const timestamp = (day) => new Date(2026, 9, day, 10).getTime() / 1000;
  assert.equal(settledPeriod({ updated_at: timestamp(1), settled_at: timestamp(9) }, now), 'Today');
  assert.equal(settledPeriod({ updated_at: timestamp(8) }, now), 'This week');
  assert.equal(settledPeriod({ updated_at: timestamp(4) }, now), 'Earlier');
});
