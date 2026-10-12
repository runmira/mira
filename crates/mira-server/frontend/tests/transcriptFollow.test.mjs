import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

/** Load the real module straight from source — no bundler, no build step. */
const load = async (path) => {
  const source = ts.transpileModule(readFileSync(new URL(path, import.meta.url), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.ESNext },
  }).outputText;
  return import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
};

const { nextFollow, distanceFromBottom, FOLLOW_ENGAGE_AT, FOLLOW_RELEASE_AT } = await load(
  '../src/lib/transcriptFollow.ts',
);

test('follow engages only inside the bottom band', () => {
  assert.equal(nextFollow(false, 0, true), true);
  assert.equal(nextFollow(false, FOLLOW_ENGAGE_AT - 1, true), true);
  assert.equal(nextFollow(false, 500, true), false);
});

test('scrolling down past the engage line does not latch follow on', () => {
  // The regression. Reading downward through a long streamed turn, the
  // reader briefly passes through the bottom band on the way to older
  // content. A single 80px threshold turns follow on there, and the next
  // token hard-pins the pane to the bottom — past everything they were
  // scrolling toward. The release band has to be wider than the engage
  // band, so drifting near the bottom never captures the pane.
  for (const distance of [FOLLOW_ENGAGE_AT, 100, 150, FOLLOW_RELEASE_AT]) {
    assert.equal(nextFollow(false, distance, true), false, `distance ${distance}`);
  }
});

test('follow survives drift inside the release band', () => {
  // Once following, small movement as tokens land must not drop it, or
  // follow stutters on and off for the whole turn.
  assert.equal(nextFollow(true, 100, true), true);
  assert.equal(nextFollow(true, FOLLOW_RELEASE_AT, true), true);
});

test('following releases only once clearly away from the bottom', () => {
  assert.equal(nextFollow(true, FOLLOW_RELEASE_AT + 1, true), false);
  assert.equal(nextFollow(true, 900, true), false);
});

test('follow can be re-acquired after a release', () => {
  let state = true;
  state = nextFollow(state, 900, true);
  assert.equal(state, false, 'releases when scrolled well up');
  state = nextFollow(state, 10, true);
  assert.equal(state, true, 're-engages at the bottom');
});

test('the disabled setting blocks engaging but does not force a release', () => {
  // Auto-follow off must still let an explicit jump-to-latest keep working,
  // so the preference is only consulted when engaging.
  assert.equal(nextFollow(false, 10, false), false);
  assert.equal(nextFollow(true, 10, false), true);
  assert.equal(nextFollow(true, 900, false), false);
});

test('the engage and release bands do not overlap', () => {
  assert.ok(
    FOLLOW_RELEASE_AT > FOLLOW_ENGAGE_AT,
    `hysteresis needs a gap: release ${FOLLOW_RELEASE_AT} must exceed engage ${FOLLOW_ENGAGE_AT}`,
  );
});

test('distance from the bottom is measured from the content edge', () => {
  // scrollTop is clamped to scrollHeight - clientHeight, so at rest the
  // distance is exactly 0 and never negative.
  assert.equal(
    distanceFromBottom({ scrollHeight: 1000, scrollTop: 1000 - 300, clientHeight: 300 }),
    0,
  );
  assert.equal(distanceFromBottom({ scrollHeight: 1000, scrollTop: 500, clientHeight: 300 }), 200);
});