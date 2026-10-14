import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { createAppRuntime, createStreamService, reconcileQueue, applyResolution, createDiagnostics, ResourceError, retryResource } = await bundle(`
export { createAppRuntime } from './src/state/createAppRuntime';
export { createStreamService } from './src/state/streamService';
export { reconcileQueue } from './src/state/queueReconciliation';
export { applyResolution } from './src/state/requestReducer';
export { createDiagnostics } from './src/state/diagnostics';
export { ResourceError, retryResource } from './src/state/resources/resourceService';
`);
const runtimes = [];
afterEach(async () => { for (const runtime of runtimes.splice(0)) await runtime.dispose(); });
function runtime(account = 'one') {
  const value = createAppRuntime({ backend: 'https://mira.test', account }); runtimes.push(value);
  value.reportConnection('open'); value.turnCommands.hydrate('a', true, []);
  return value;
}
function scheduler() {
  let id = 0; let hidden = false;
  const frames = new Map(); const timers = new Map();
  return { frames, timers, hide() { hidden = true; },
    api: { frame: cb => { frames.set(++id, cb); return id; }, cancelFrame: id => frames.delete(id),
      timer: (cb, delay) => { timers.set(++id, { cb, delay }); return id; }, cancelTimer: id => timers.delete(id), hidden: () => hidden },
    frame() { [...frames.values()][0]?.(); }, timer() { [...timers.values()][0]?.cb(); },
  };
}
const pendingTool = id => ({ kind: 'tool', call: { id, type: 'function', function: { name: 'bash', arguments: '{}' } }, status: 'pending', preview: null, result: null });

test('text ordering, repeated deltas, frame batching, hidden fallback and disposal', () => {
  const scheduled = scheduler(); const entries = new Map([['a', []], ['b', []]]); let writes = 0;
  let identity = { session: 'a', generation: 1 };
  const streams = createStreamService(() => identity, (owner, update) => { writes++; entries.set(owner.session, update(entries.get(owner.session))); }, scheduled.api);
  streams.queueToken('ha'); streams.queueToken('ha');
  assert.equal(scheduled.frames.size, 1); assert.equal(scheduled.timers.size, 1);
  scheduled.frame(); assert.equal(entries.get('a')[0].msg.content, 'haha'); assert.equal(writes, 1);
  streams.queueToken(' tail'); streams.flush();
  identity = { session: 'b', generation: 2 }; scheduled.hide();
  streams.queueToken('B'); assert.equal(scheduled.frames.size, 0); assert.equal([...scheduled.timers.values()][0].delay, 50);
  scheduled.timer(); assert.equal(entries.get('a')[0].msg.content, 'haha tail'); assert.equal(entries.get('b')[0].msg.content, 'B');
  streams.queueToken('never'); streams.dispose(); assert.equal(scheduled.frames.size + scheduled.timers.size, 0);
  streams.flush(); assert.equal(entries.get('b')[0].msg.content, 'B');
});

test('terminal transition drains final tokens before publishing idle, with effects after commit', () => {
  const app = runtime(); const snapshots = []; const effects = [];
  globalThis.requestAnimationFrame = () => 1; globalThis.cancelAnimationFrame = () => {};
  try {
    app.streams.queueToken('final');
    const sub = app.sessionState.subscribe(state => snapshots.push(state.sessions.get('a')));
    app.transition(() => {
      app.streams.flush(); app.turnCommands.reportTurnRunning(false);
      app.effect(() => effects.push(app.readSession().busy));
    });
    assert.equal(snapshots.length, 1); assert.equal(snapshots[0].busy, false);
    assert.equal(snapshots[0].thinking, false); assert.equal(snapshots[0].entries[0].msg.content, 'final');
    assert.deepEqual(effects, [false]); sub.unsubscribe();
  } finally { delete globalThis.requestAnimationFrame; delete globalThis.cancelAnimationFrame; }
});

test('approval submission never resolves locally, double click is blocked, server resolution is atomic', () => {
  const app = runtime(); app.updateSessionData('entries', [pendingTool('p')]);
  let sent = 0;
  const frame = { type: 'approve', call_id: 'p', allow: true, scope: 'once' };
  assert.equal(app.requests.submit('p', frame, () => { sent++; return true; }), true);
  assert.equal(app.requests.submit('p', frame, () => { sent++; return true; }), false);
  assert.equal(sent, 1); assert.equal(app.readSession().entries[0].status, 'pending');
  let commits = 0;
  const sub = app.sessionState.subscribe(state => { commits++;
    assert.equal(state.sessions.get('a').entries[0].status, 'running');
    assert.equal(state.sessions.get('a').requestDelivery.p, undefined);
  });
  app.transition(() => { app.requests.resolve('p'); app.updateSessionData('entries', previous => applyResolution(previous, { type: 'approval_resolved', call_id: 'p', allow: true })); });
  assert.equal(commits, 1); sub.unsubscribe();
  assert.equal(app.requests.submit('p', frame, () => true), false);
});

test('failed submission is retryable; disconnect keeps running work and fails pending decisions', () => {
  const app = runtime(); app.updateSessionData('entries', [pendingTool('p')]);
  const frame = { type: 'approve', call_id: 'p', allow: false, scope: 'once' };
  assert.equal(app.requests.submit('p', frame, () => false), false);
  assert.equal(app.readSession().requestDelivery.p.status, 'failed');
  assert.equal(app.requests.submit('p', frame, () => true), true);
  app.requests.disconnected(); app.reportConnection('closed');
  assert.equal(app.readSession().busy, true);
  assert.equal(app.readSession().entries[0].status, 'pending');
  assert.equal(app.readSession().requestDelivery.p.status, 'failed');
});

test('scope isolation, invalidation bursts, unabortable old reads and authoritative settings updates', async () => {
  const app = runtime(); const other = runtime('two');
  assert.notDeepEqual(app.resources.key('engines'), other.resources.key('engines'));
  const original = globalThis.fetch; let calls = 0;
  try {
    globalThis.fetch = async () => { calls++; return new Response(JSON.stringify({ engines: [], fresh: true })); };
    await Promise.all(Array.from({ length: 20 }, () => app.resources.refresh('engines')));
    assert.equal(calls, 1);
    let finish;
    globalThis.fetch = () => new Promise(resolve => { finish = resolve; });
    const oldKey = app.resources.key('gitStatus');
    const pending = app.queryClient.fetchQuery(app.resources.options('gitStatus')).catch(() => undefined);
    app.turnCommands.hydrate('b', false, []); app.resources.cancelContext();
    finish(new Response(JSON.stringify({ branch: 'old' }))); await pending;
    assert.equal(app.queryClient.getQueryData(oldKey), undefined);
    let resolveSettings;
    globalThis.fetch = () => new Promise(resolve => { resolveSettings = resolve; });
    const settingsRead = app.queryClient.fetchQuery(app.resources.options('settings')).catch(() => undefined);
    app.resources.accept('settings', { configured: true, default_model: 'new' });
    resolveSettings(new Response(JSON.stringify({ configured: false, default_model: 'old' }))); await settingsRead;
    assert.equal(app.queryClient.getQueryData(app.resources.key('settings')).default_model, 'new');
  } finally { globalThis.fetch = original; }
});

test('reads retry only transient failures; secrets never enter request state or diagnostics', () => {
  assert.equal(retryResource(0, new ResourceError(503)), true);
  assert.equal(retryResource(1, new ResourceError(503)), false);
  for (const status of [400, 401, 403, 404]) assert.equal(retryResource(0, new ResourceError(status)), false);
  const app = runtime(); app.updateSessionData('secretRequests', [{ promptId: 's', name: 'key', reason: 'needed' }]);
  app.requests.submit('s', { type: 'prompt_response', kind: 'secret', prompt_id: 's', value: 'SECRET_SENTINEL', cancelled: false }, () => true);
  assert.equal(JSON.stringify(app.readSession()).includes('SECRET_SENTINEL'), false);
  const diagnostics = createDiagnostics(true);
  for (let i = 0; i < 200; i++) diagnostics.record({ type: 'token', session: 'a', generation: 1, at: i, text: 'SECRET_SENTINEL' });
  assert.equal(diagnostics.read().events.length, 128); assert.equal(diagnostics.read().counts.token, 200);
  assert.equal(JSON.stringify(diagnostics.read()).includes('SECRET_SENTINEL'), false);
});

test('queue snapshots preserve uncertain delivery, reconcile by ID and allow repeated content', () => {
  const local = [{ id: 'a', text: 'repeat' }];
  assert.equal(reconcileQueue(local, [])[0].text, 'repeat');
  const confirmed = [{ id: 'a', text: 'repeat', fingerprint: 'accepted' }];
  assert.equal(reconcileQueue(local, confirmed).length, 1);
  assert.equal(reconcileQueue(local, confirmed)[0].accepted, true);
  assert.equal(reconcileQueue(local, [{ id: 'b', text: 'repeat', fingerprint: 'accepted' }]).length, 2);
  assert.deepEqual(reconcileQueue(local, [], new Set(['a'])), []);
});

test('versioned persistence drops unknown/sensitive fields and scoped drafts do not share IDs', () => {
  const values = new Map([
    ['mira.workspace-ui.v1', JSON.stringify({ version: 1, values: { contextOpen: true, transcript: 'SECRET_SENTINEL' } })],
    ['mira.preferences.v1', JSON.stringify({ version: 1, values: { 'mira.theme': 'dark', secret: 'SECRET_SENTINEL', 'mira.sound.turnDone': 'invalid' } })],
  ]);
  const storage = { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) };
  const app = createAppRuntime({ backend: 'https://one.test', account: 'one' }, storage); runtimes.push(app);
  const other = createAppRuntime({ backend: 'https://one.test', account: 'two' }, storage); runtimes.push(other);
  app.commands.setContextOpen(false); app.preferences.set('mira.theme', 'light');
  assert.equal(values.get('mira.workspace-ui.v1').includes('SECRET_SENTINEL'), false);
  assert.equal(values.get('mira.preferences.v1').includes('SECRET_SENTINEL'), false);
  assert.equal(app.preferences.source.get()['mira.sound.turnDone'], undefined);
  assert.notEqual(app.drafts.key('same'), other.drafts.key('same'));
  app.drafts.update(app.drafts.key('same'), { text: 'one', attachments: [], images: [] });
  assert.equal(other.drafts.get(other.drafts.key('same')).text, '');
});

test('saved defaults do not replace a session selection before an authoritative session event', async () => {
  const app = runtime(); app.updateSessionData('model', 'session-model');
  const original = globalThis.fetch; let pending = 0; let maximum = 0;
  try {
    globalThis.fetch = async (_url, init) => {
      pending++; maximum = Math.max(maximum, pending);
      const update = JSON.parse(init.body); await Promise.resolve(); pending--;
      return new Response(JSON.stringify({ configured: true, default_model: update.default_model }));
    };
    await Promise.all([app.resources.saveSettings({ default_model: 'first' }), app.resources.saveSettings({ default_model: 'second' })]);
    assert.equal(maximum, 1); assert.equal(app.queryClient.getQueryData(app.resources.key('settings')).default_model, 'second');
    assert.equal(app.readSession().model, 'session-model');
  } finally { globalThis.fetch = original; }
});

test('Stop drains final tokens atomically and a failed transport does not imply Stop was accepted', () => {
  const app = runtime();
  globalThis.requestAnimationFrame = () => 1; globalThis.cancelAnimationFrame = () => {};
  try {
    app.service.refs.wsRef.current = { sendImmediate: () => false, close() {} };
    assert.equal(app.requestStop(), false); assert.equal(app.readSession().busy, true);
    app.streams.queueToken('final before Stop');
    app.service.refs.wsRef.current = { sendImmediate: () => true, close() {} };
    const snapshots = []; const sub = app.sessionState.subscribe(state => snapshots.push(state.sessions.get('a')));
    app.requestStop(); assert.equal(snapshots.length, 1);
    assert.equal(snapshots[0].busy, false); assert.equal(snapshots[0].stopRequested, true);
    assert.equal(snapshots[0].entries[0].msg.content, 'final before Stop');
    app.turnCommands.reportTurnRunning(false); assert.equal(app.readSession().stopRequested, false);
    sub.unsubscribe();
  } finally { delete globalThis.requestAnimationFrame; delete globalThis.cancelAnimationFrame; }
});

test('failed unsent request ownership cannot route a reused ID to the old session', () => {
  const r=runtime();r.updateSessionData('entries',[pendingTool('same')]);
  assert.equal(r.requests.submit('same',{type:'approve',call_id:'same',allow:true},()=>false),false);
  r.turnCommands.hydrate('b',false,[]);r.updateSessionData('entries',[pendingTool('same')]);
  r.requests.resolve('same');
  assert.ok(r.readSession().resolvedRequests.same);
  assert.ok(!r.sessionState.get().sessions.get('a').resolvedRequests.same);
});


test('in-flight request IDs retain their owner across a session switch', () => {
  const r=runtime(); const frame={type:'approve',call_id:'same',allow:true};
  r.updateSessionData('entries',[pendingTool('same')]);
  assert.equal(r.requests.submit('same',frame,()=>true),true);
  r.turnCommands.hydrate('b',false,[]);r.updateSessionData('entries',[pendingTool('same')]);
  let sent=false;
  assert.equal(r.requests.submit('same',frame,()=>{sent=true;return true;}),false);
  assert.equal(sent,false);
  r.requests.resolve('same');
  assert.ok(r.sessionState.get().sessions.get('a').resolvedRequests.same);
  assert.equal(r.readSession().resolvedRequests.same,undefined);
  assert.equal(r.sessionState.get().sessions.get('a').requestDelivery.same,undefined);
  assert.equal(r.requests.submit('same',frame,()=>true),true);
  r.requests.fail('same','retry');
  assert.equal(r.readSession().requestDelivery.same.status,'failed');
});

test('Ready reconciliation preserves resumed turn identities and usage bases per session', () => {
  const r=runtime();const base={turn:0,base:{input_tokens:12,output_tokens:3}};
  r.sessionServices.ref('turnBase').current=base;
  r.beginTerminalTurn('a-first',10,0);r.beginTerminalTurn('a-next',20,1);
  r.turnCommands.hydrate('b',true,[]);r.beginTerminalTurn('b-first',30,0);
  r.reconcileTerminals(true);
  assert.equal(r.acceptTerminal().id,'b-first');
  r.turnCommands.hydrate('a',true,[]);r.reconcileTerminals(true);
  const first=r.acceptTerminal();
  assert.equal(first.id,'a-first');assert.equal(first.startedAt,10);
  assert.equal(first.usageBase,base);assert.equal(first.pending,true);
  assert.equal(r.acceptTerminal().id,'a-next');
});

test('Ready discards snapshot-confirmed completed turns and clears idle-session terminals', () => {
  const r=runtime();r.beginTerminalTurn('finished',10,0);r.beginTerminalTurn('live',20,1);
  r.reconcileTerminals(true,[{started_at:10,ended_at:15},{started_at:20,ended_at:null}]);
  assert.equal(r.acceptTerminal().id,'live');
  r.beginTerminalTurn('stale',30,2);r.reconcileTerminals(false);
  assert.equal(r.acceptTerminal().id,undefined);
});

test('engine discovery is available in settings without an attached chat or an open websocket', async () => {
  const app = runtime(); const original = globalThis.fetch;
  globalThis.fetch = async () => new Response(JSON.stringify({engines:[],fresh:true}));
  try {
    const options = app.resources.options('engines');
    const result = await options.queryFn({signal:new AbortController().signal});
    assert.equal(result.fresh,true);
  } finally { globalThis.fetch = original; }
});
