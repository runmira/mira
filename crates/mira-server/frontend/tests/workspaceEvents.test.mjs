import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { useApprovalEvents, useTranscriptEvents, useSessionEvents, useStreamCommands } =
  await bundle(
    `
export { useApprovalEvents } from './src/app/useApprovalEvents';
export { useTranscriptEvents } from './src/app/useTranscriptEvents';
export { useSessionEvents } from './src/app/useSessionEvents';
export { useStreamCommands } from './src/app/useStreamCommands';
`,
    {
      'src/app/shared': 'export const useStableCallback = fn => fn;',
      'src/lib/attention': 'export const callForAttention = () => {};',
      'src/lib/agentTerminal':
        'export const commandStart = () => {}; export const commandEnd = () => {}; export const clear = () => {}; export const reset = () => {}; export const outputLine = () => {};',
      'src/components/VirtualTranscript': 'export const hasTranscriptPosition = () => false;',
    },
  );
const ref = (current) => ({ current });
function harness() {
  let entries = [];
  const context = {
    setEntries: (update) => {
      entries = typeof update === 'function' ? update(entries) : update;
    },
    pendingProposalsRef: ref(new Map()),
    pendingAskUserRef: ref(new Map()),
    approvalStarted: ref({}),
    chatTitleRef: ref('Test'),
    wsRef: ref(null),
    tokenBufRef: ref(''),
    tokenRafRef: ref(null),
    nativeFramesRef: ref([]),
    nativeRafRef: ref(null),
    playPing() {},
    setThinking() {},
    clearThinkingIdle() {},
    scheduleThinkingIdle() {},
  };
  return { context, entries: () => entries };
}

test('a plan arriving before its tool start is attached once the tool arrives', () => {
  const h = harness();
  const proposal = { title: 'Plan', steps: [] };
  useApprovalEvents(h.context)({ type: 'plan_request', prompt_id: 'p1', plan: proposal });
  assert.equal(h.context.pendingProposalsRef.current.get('p1'), proposal);
  useTranscriptEvents(h.context)({
    type: 'tool_start',
    call: { id: 'p1', function: { name: 'plan', arguments: '{}' } },
  });
  assert.equal(h.entries()[0].plan.proposal, proposal);
  assert.equal(h.context.pendingProposalsRef.current.size, 0);
});

test('late approval-rule responses do not revive a resolved approval', () => {
  let applied = 0;
  useApprovalEvents({ approvalStarted: ref({}), setApprovalRules: () => applied++ })({
    type: 'approval_rules',
    call_id: 'old',
    rules: [],
    error: null,
  });
  assert.equal(applied, 0);
});

test('queued deliveries only dispatch to the attached session', () => {
  const delivered = [];
  const handle = useSessionEvents({
    sessionIdRef: ref('active'),
    sendNow: (...args) => delivered.push(args),
  });
  const item = { id: 'q1', text: 'Queued task', images: [] };
  handle({ type: 'queue_delivery', session_id: 'other', item });
  assert.deepEqual(delivered, []);
  handle({ type: 'queue_delivery', session_id: 'active', item });
  assert.deepEqual(delivered, [['Queued task', [], false, 'q1']]);
});

test('stale history pages cannot clear the active history request', () => {
  let applied = 0;
  const request = ref('new-request');
  const handle = useSessionEvents({
    sessionIdRef: ref('active'),
    historyRequestRef: request,
    setHistoryLoading: () => applied++,
  });
  handle({
    type: 'history_page',
    session_id: 'active',
    request_id: 'old-request',
    page: null,
    error: null,
  });
  handle({
    type: 'history_page',
    session_id: 'other',
    request_id: 'new-request',
    page: null,
    error: null,
  });
  assert.equal(request.current, 'new-request');
  assert.equal(applied, 0);
});

test('native frames flush buffered provider text before the next event', () => {
  const h = harness();
  globalThis.document = { hidden: false };
  globalThis.requestAnimationFrame = () => 1;
  globalThis.cancelAnimationFrame = () => {};
  h.context.tokenBufRef.current = 'Provider text';
  const order = [];
  const stream = useStreamCommands({
    ...h.context,
    handleMessage: () =>
      order.push(
        h
          .entries()
          .map((e) => e.msg?.content)
          .filter(Boolean),
      ),
  });
  stream.onMessage({ type: 'acp_text', text: 'Native text' });
  assert.equal(h.context.tokenBufRef.current, '');
  assert.equal(h.context.nativeFramesRef.current.length, 1);
  stream.onMessage({ type: 'mode_changed', mode: 'manual' });
  assert.equal(h.context.nativeFramesRef.current.length, 0);
  assert.equal(order.length, 1);
  assert.match(order[0].join(''), /Provider text/);
  delete globalThis.document;
  delete globalThis.requestAnimationFrame;
  delete globalThis.cancelAnimationFrame;
});
