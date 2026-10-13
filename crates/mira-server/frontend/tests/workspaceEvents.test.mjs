import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { createAppRuntime, useApprovalEvents, useTranscriptEvents, useSessionEvents, useStreamCommands, useComposerCommands } = await bundle(`
export { createAppRuntime } from './src/state/createAppRuntime';
export { useApprovalEvents } from './src/app/useApprovalEvents';
export { useTranscriptEvents } from './src/app/useTranscriptEvents';
export { useSessionEvents } from './src/app/useSessionEvents';
export { useStreamCommands } from './src/app/useStreamCommands';
export { useComposerCommands } from './src/app/useComposerCommands';
`, {
  'src/state/RuntimeProvider': 'export const useAppRuntime = () => globalThis.eventRuntime; export const useOptionalRuntime = () => null;',
  'src/app/shared': 'export const useStableCallback = fn => fn;',
  'src/lib/attention': 'export const callForAttention = () => {};',
  'src/lib/agentTerminal': 'export const commandStart = () => {}; export const commandEnd = () => {}; export const clear = () => {}; export const reset = () => {}; export const outputLine = () => {};',
});
const ref = current => ({ current });
const runtimes = [];
afterEach(async () => { for (const runtime of runtimes.splice(0)) await runtime.dispose(); delete globalThis.eventRuntime; });
function harness() {
  const runtime = createAppRuntime({ backend: 'https://mira.test', account: 'test' });
  runtimes.push(runtime); globalThis.eventRuntime = runtime;
  runtime.turnCommands.hydrate('active', true, []);
  const context = {
    updateTranscript: update => runtime.updateSessionData('entries', update),
    pendingProposalsRef: runtime.sessionServices.ref('pendingProposals'),
    pendingAskUserRef: runtime.sessionServices.ref('pendingQuestions'),
    approvalStarted: runtime.sessionServices.ref('approvalStarted'),
    chatTitleRef: ref('Test'), wsRef: ref(null), streams: runtime.streams,
    getSessionId: () => 'active', isTurnRunning: runtime.isTurnRunning,
    playPing() {}, reportThinking: runtime.turnCommands.reportThinking,
    clearThinkingIdle() {}, scheduleThinkingIdle() {},
  };
  return { runtime, context, entries: () => runtime.readSession().entries };
}

test('a plan arriving before its tool start is attached once the tool arrives', () => {
  const h = harness(); const proposal = { title: 'Plan', steps: [] };
  useApprovalEvents(h.context)({ type: 'plan_request', prompt_id: 'p1', plan: proposal });
  assert.equal(h.context.pendingProposalsRef.current.get('p1'), proposal);
  useTranscriptEvents(h.context)({ type: 'tool_start', call: { id: 'p1', function: { name: 'plan', arguments: '{}' } } });
  assert.equal(h.entries()[0].plan.proposal, proposal);
  assert.equal(h.context.pendingProposalsRef.current.size, 0);
});

test('resolution before tool-start is retained and duplicate requests do not reopen it', () => {
  const h = harness(); const proposal = { title: 'Plan', steps: [] };
  const approval = useApprovalEvents(h.context);
  approval({ type: 'plan_request', prompt_id: 'p1', plan: proposal });
  approval({ type: 'prompt_resolved', kind: 'plan', prompt_id: 'p1', approved: false });
  useTranscriptEvents(h.context)({ type: 'tool_start', call: { id: 'p1', function: { name: 'plan', arguments: '{}' } } });
  assert.equal(h.entries()[0].plan.decision.approved, false);
  approval({ type: 'plan_request', prompt_id: 'p1', plan: proposal });
  assert.equal(h.entries()[0].plan.decision.approved, false);
  assert.equal(h.entries().length, 1);
});

test('late approval-rule responses do not revive a resolved approval', () => {
  harness(); let applied = 0;
  useApprovalEvents({ approvalStarted: ref({}), recordApprovalRules: () => applied++ })({ type: 'approval_rules', call_id: 'old', rules: [], error: null });
  assert.equal(applied, 0);
});

test('queued deliveries only dispatch to the attached session', () => {
  const h = harness(); const delivered = [];
  const handle = useSessionEvents({ getSessionId: () => 'active', sendNow: (...args) => delivered.push(args) });
  const item = { id: 'q1', text: 'Queued task', images: [] };
  handle({ type: 'queue_delivery', session_id: 'other', item }); assert.deepEqual(delivered, []);
  handle({ type: 'queue_delivery', session_id: 'active', item });
  assert.deepEqual(delivered, [['Queued task', [], false, 'q1']]);
  assert.equal(h.runtime.sessionState.get().active, 'active');
});

test('stale history pages cannot clear the active history request', () => {
  harness(); let applied = 0; const request = ref('new-request');
  const handle = useSessionEvents({ getSessionId: () => 'active', historyRequestRef: request, recordHistoryLoading: () => applied++ });
  handle({ type: 'history_page', session_id: 'active', request_id: 'old-request', page: null, error: null });
  handle({ type: 'history_page', session_id: 'other', request_id: 'new-request', page: null, error: null });
  assert.equal(request.current, 'new-request'); assert.equal(applied, 0);
});

test('native frames flush buffered provider text before a structural event', () => {
  const h = harness();
  globalThis.document = { hidden: false };
  globalThis.requestAnimationFrame = () => 1;
  globalThis.cancelAnimationFrame = () => {};
  try {
    h.runtime.streams.queueToken('Provider text');
    const order = [];
    const stream = useStreamCommands({ ...h.context, handleMessage: () => {
      h.runtime.streams.flush(); order.push(h.entries().map(entry => entry.msg?.content).filter(Boolean));
    } });
    stream.onMessage({ type: 'acp_text', text: 'Native text' });
    stream.onMessage({ type: 'mode_changed', mode: 'manual' });
    assert.equal(order.length, 1); assert.match(order[0].join(''), /Provider text.*Native text/);
  } finally { delete globalThis.document; delete globalThis.requestAnimationFrame; delete globalThis.cancelAnimationFrame; }
});

test('normal sends use the durable outbox and keep the draft until acceptance without duplicate submissions', () => {
  const h = harness(); const frames = [];
  const key = h.runtime.drafts.key('active');
  h.runtime.drafts.update(key, { text: 'Keep this draft', attachments: [], images: [] });
  const commands = useComposerCommands({ ...h.context, updateQueueProjection: h.runtime.updateQueues,
    wsRef: ref({ sendImmediate: message => { frames.push(message); return true; } }) });
  assert.equal(commands.onSend('Keep this draft'), false);
  assert.equal(h.runtime.drafts.get(key).text, 'Keep this draft');
  commands.onSend('Keep this draft'); assert.equal(frames.length, 1);
  assert.equal(frames[0].type, 'queue_input');
  const accepted = { id: frames[0].id, text: 'Keep this draft', images: [], fingerprint: 'known', dispatching: false, error: null };
  const events = useSessionEvents({ ...h.context, updateSessionQueue: commands.updateSessionQueue,
    recordQueueRecovery: h.runtime.updateRecovery });
  h.runtime.transition(() => events({ type: 'queue_updated', session_id: 'active', items: [accepted] }));
  assert.equal(h.runtime.drafts.get(key).text, '');
  assert.equal(h.runtime.sessionState.get().queued.get('active')[0].accepted, true);
});

test('a late queue acceptance does not erase a newer draft', () => {
  const h = harness(); const key = h.runtime.drafts.key('active');
  h.runtime.drafts.update(key, { text: 'first', attachments: [], images: [] });
  h.runtime.drafts.markDelivery(key, 'id');
  h.runtime.drafts.update(key, { text: 'newer', attachments: [], images: [] });
  h.runtime.drafts.confirmDelivery('id');
  assert.equal(h.runtime.drafts.get(key).text, 'newer');
});
