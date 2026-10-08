import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const { outputText } = ts.transpileModule(readFileSync(new URL('../ws.ts', import.meta.url), 'utf8'), { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } });
const { connect } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
test('conditional queue changes never replay offline or before reattachment', () => {
  const original = { WebSocket: globalThis.WebSocket, location: globalThis.location, window: globalThis.window, document: globalThis.document, setTimeout: globalThis.setTimeout };
  const sockets = [], reconnects = [];
  class Socket {
    static OPEN = 1;
    readyState = 0;
    sent = [];
    constructor() { sockets.push(this); }
    send(body) { this.sent.push(JSON.parse(body)); }
    close() { this.readyState = 3; }
  }
  globalThis.WebSocket = Socket;
  globalThis.location = { protocol: 'http:', host: 'localhost:8797' };
  globalThis.window = globalThis.document = { addEventListener() {}, removeEventListener() {} };
  globalThis.setTimeout = callback => { reconnects.push(callback); return 1; };
  const mutation = { type: 'reorder_queued_input', session_id: 'chat', request_id: 'one', id: 'message', before_id: null };
  let client;
  try {
    client = connect(() => {}, () => {});
    assert.equal(client.sendImmediate(mutation), false);
    sockets[0].readyState = Socket.OPEN; sockets[0].onopen();
    assert.deepEqual(sockets[0].sent.map(frame => frame.type), ['sync']);
    assert.equal(client.sendImmediate(mutation), true);
    client.setSession('chat');
    sockets[0].readyState = 3; sockets[0].onclose(); reconnects.shift()();
    sockets[1].readyState = Socket.OPEN; sockets[1].onopen();
    assert.equal(client.sendImmediate(mutation), false);
    assert.deepEqual(sockets[1].sent.map(frame => frame.type), ['attach']);
    sockets[1].onmessage({ data: JSON.stringify({ type: 'ready', session_id: 'chat' }) });
    assert.equal(client.sendImmediate(mutation), true);
  } finally { client?.close(); Object.assign(globalThis, original); }
});
