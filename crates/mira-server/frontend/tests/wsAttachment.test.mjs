import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { connect } = await bundle(`export { connect } from './src/ws';`);

function harness() {
  const sockets = [];
  const timers = new Map();
  let nextTimer = 0;
  const originals = Object.fromEntries(['WebSocket', 'window', 'document', 'location', 'setTimeout', 'clearTimeout'].map(key => [key, globalThis[key]]));
  class Socket {
    static OPEN = 1;
    readyState = 0;
    sent = [];
    constructor() { sockets.push(this); }
    send(data) { this.sent.push(JSON.parse(data)); }
    close() { this.readyState = 3; this.onclose?.(); }
    open() { this.readyState = 1; this.onopen?.(); }
    message(msg) { this.onmessage?.({ data: JSON.stringify(msg) }); }
  }
  globalThis.WebSocket = Socket;
  globalThis.window = { addEventListener() {}, removeEventListener() {} };
  globalThis.document = { hidden: false, addEventListener() {}, removeEventListener() {} };
  globalThis.location = { protocol: 'https:', host: 'mira.test' };
  globalThis.setTimeout = callback => { const id = ++nextTimer; timers.set(id, callback); return id; };
  globalThis.clearTimeout = id => timers.delete(id);
  const received = [];
  const client = connect(msg => received.push(msg), () => {});
  return { sockets, timers, received, client, restore() { client.close(); Object.assign(globalThis, originals); } };
}

test('attach accepts only target Ready and global activity until its snapshot arrives', () => {
  const h = harness();
  try {
    const socket = h.sockets[0]; socket.open();
    socket.message({ type: 'ready', session_id: 'a' });
    h.client.setSession('a');
    h.client.attach('b');
    socket.message({ type: 'token', text: 'old' });
    socket.message({ type: 'ready', session_id: 'a' });
    socket.message({ type: 'session_activity_snapshot', snapshot: { epoch: 'e', revision: 1, running: ['a'] } });
    socket.message({ type: 'ready', session_id: 'b' });
    socket.message({ type: 'token', text: 'new' });
    assert.deepEqual(h.received.map(msg => msg.type), ['ready', 'session_activity_snapshot', 'ready', 'token']);
    assert.equal(h.received.at(-1).text, 'new');
  } finally { h.restore(); }
});

test('disconnect during attach reconnects to target and ignores frames from the old socket', () => {
  const h = harness();
  try {
    const old = h.sockets[0]; old.open(); h.client.attach('b'); old.close();
    const timer = [...h.timers.values()][0]; h.timers.clear(); timer();
    const current = h.sockets[1]; current.open();
    assert.deepEqual(current.sent[0], { type: 'attach', session_id: 'b' });
    old.message({ type: 'token', text: 'stale' });
    old.onclose();
    current.message({ type: 'ready', session_id: 'default' });
    current.message({ type: 'ready', session_id: 'b' });
    assert.deepEqual(h.received, [{ type: 'ready', session_id: 'b' }]);
    assert.equal(h.timers.size, 0);
    current.close(); assert.equal(h.timers.size, 1);
    h.client.close(); assert.equal(h.timers.size, 0);
    current.message({ type: 'done' });
    assert.equal(h.received.length, 1);
  } finally { h.restore(); }
});

test('attach before socket open and rapid selection accept only the latest target', () => {
  const h = harness();
  try {
    h.client.attach('a');
    h.client.attach('b');
    const socket = h.sockets[0]; socket.open();
    assert.deepEqual(socket.sent[0], { type: 'attach', session_id: 'b' });
    socket.message({ type: 'ready', session_id: 'a' });
    socket.message({ type: 'token', text: 'old' });
    socket.message({ type: 'ready', session_id: 'b' });
    assert.deepEqual(h.received, [{ type: 'ready', session_id: 'b' }]);
  } finally { h.restore(); }
});
