import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

/** Load a dependency-free TS module straight from source, the way
 *  `chatFailure.test.mjs` does — no bundler, no build step. */
const load = async (path) => {
  const source = ts.transpileModule(readFileSync(new URL(path, import.meta.url), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.ESNext },
  }).outputText;
  return import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
};

const stops = await load('../src/lib/acpStop.ts');
const chat = await load('../src/lib/chatFailure.ts');

/** The filter `TurnView` uses to decide what counts as a failure worth an
 *  error card. Mirrored here so the test fails if that filter changes. */
const escalatesToErrorCard = (e) =>
  e.kind === 'error' || (e.kind === 'warning' && chat.isFatalChatWarning(e.text));

test('a turn the user stopped is recognised, on either spelling', () => {
  assert.equal(stops.isUserCancelledStop('cancelled'), true);
  assert.equal(stops.isUserCancelledStop('canceled'), true);
});

test('only a user stop counts as a user stop', () => {
  // A failure the agent chose must NOT be softened into "you stopped it" —
  // that would hide a real fault behind a message implying the user's own
  // action caused it.
  for (const stop of ['end_turn', 'refusal', 'max_tokens', 'error', 'cancelled_by_agent', '']) {
    assert.equal(stops.isUserCancelledStop(stop), false, stop);
  }
});

test('a stopped turn reads as the user\'s own doing', () => {
  assert.equal(stops.describeAcpStop('cancelled'), 'Turn ended manually by user.');
  assert.equal(stops.describeAcpStop('canceled'), 'Turn ended manually by user.');
});

test('a stopped turn is not a success — it produced no answer', () => {
  // Deliberately NOT in the success allowlist: it needs a note, not silence.
  assert.equal(stops.isSuccessfulAcpStop('cancelled'), false);
  assert.equal(stops.isSuccessfulAcpStop('end_turn'), true);
});

test('the manual-stop note renders as a plain line, never an error card', () => {
  // This is the regression. The old code appended this as
  // `{kind:'error', responseFailure:true}`, which put it through the same
  // gates as a provider outage and rendered the "The request could not
  // finish / Retry message / Connection settings" card for a turn the user
  // had deliberately stopped.
  const note = { kind: 'warning', text: stops.describeAcpStop('cancelled') };

  assert.equal(chat.isFatalChatWarning(note.text), false, 'must not be treated as fatal');
  assert.equal(chat.isResponseFailure(note), false, 'must not authorize a retry');
  assert.equal(escalatesToErrorCard(note), false, 'must not reach ErrorNotice');
});

test('real failures still get the actionable treatment', () => {
  // The softening must not leak: an agent-side failure is still an error.
  const failure = { kind: 'error', text: 'The agent stopped: refusal', responseFailure: true };
  assert.equal(chat.isResponseFailure(failure), true);
  assert.equal(escalatesToErrorCard(failure), true);
  assert.equal(stops.isUserCancelledStop('refusal'), false);
  assert.equal(stops.describeAcpStop('refusal'), 'The agent stopped: refusal.');
});