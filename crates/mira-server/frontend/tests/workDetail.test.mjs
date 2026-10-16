import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

const compile = (path) => ts.transpileModule(readFileSync(new URL(path, import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
}).outputText;
const url = (source) => `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`;
const items = url(compile('../src/lib/workItems.ts'));
const { workDetail, diffTotals, plainLine } = await import(url(compile('../src/lib/workDetail.ts').replace("'./workItems'", JSON.stringify(items))));

const item = {
  key: '/w#1', cwd: '/w', worktree: '/w/.mira/worktrees/x', branch: 'x', pr: { number: 1 },
  sessions: [{ id: 'a', title: 'Lead', created_at: 100, first_user_message: 'Do the thing' }, { id: 'b', title: 'Alt', created_at: 200, forked_from: 'a' }],
};
const pr = {
  body: 'Adds it.\n\nFixes #4',
  mergeable: true,
  summary: { base_ref: 'main', requested_reviewers: ['sam'] },
  checks: [{ name: 'test', status: 'completed', conclusion: 'success', url: 'u' }, { name: 'lint', status: 'queued', conclusion: null, url: null }],
  reviews: [{ author: 'kim', state: 'COMMENTED' }, { author: 'kim', state: 'APPROVED' }],
  timeline: [{ kind: 'reviewed', actor: 'kim', message: 'approved', at: '1970-01-01T00:05:00Z' }, { kind: 'x', actor: null, message: 'no time', at: null }],
};

test('a PR supplies the goal, base, checks, reviewers, files and timeline', () => {
  const d = workDetail(item, { pr, prFiles: { files: [{ filename: 'a.rs', additions: 3, deletions: 1 }] } });
  assert.equal(d.goal, 'Adds it.\n\nFixes #4');
  assert.equal(d.base, 'main');
  assert.equal(d.filesSource, 'pr');
  assert.deepEqual(d.checks.map((c) => c.state), ['passing', 'pending']);
  assert.deepEqual(d.reviewers, [{ name: 'sam', state: 'pending' }, { name: 'kim', state: 'approved' }]);
  assert.deepEqual(d.timeline.map((e) => [e.at, e.kind]), [[300, 'review'], [200, 'you'], [100, 'chat']]);
  assert.deepEqual(diffTotals(d), { added: 3, removed: 1, files: 1 });
});

test('without a PR: the first message is the goal and changes are uncommitted', () => {
  const d = workDetail({ ...item, pr: null }, {
    uncommitted: { added: 2, removed: 0, files: [{ path: 'b.ts', added: 2, removed: 0 }] },
    branch: { base: 'main', ahead: 3, behind: 1 },
  });
  assert.equal(d.goal, 'Do the thing');
  assert.equal(d.filesSource, 'uncommitted');
  assert.deepEqual([d.base, d.ahead, d.behind], ['main', 3, 1]);
  assert.equal(d.mergeable, null);
});

test('activity lines drop markdown and keep the first line', () => {
  assert.equal(plainLine("⚠️ **Mira couldn't finish this review:** provider stream [See the run](https://x/y).\n\nComment `@bot review` to retry."),
    "⚠️ Mira couldn't finish this review: provider stream See the run.");
  assert.equal(plainLine('\n## Summary\n'), 'Summary');
  assert.equal(plainLine('- `fmt` passed :tada: <img src="x">'), 'fmt passed');
  assert.equal(plainLine(''), '');
});
