import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';

const p = await bundle(`export * from './src/components/device/protocol';`);

const envelope = (tag, payload) => {
  const out = new Uint8Array(5 + payload.length);
  new DataView(out.buffer).setUint32(0, 1 + payload.length);
  out[4] = tag;
  out.set(payload, 5);
  return out;
};

test('envelopes reassemble across arbitrary chunk boundaries', () => {
  const avcc = Uint8Array.of(1, 0x64, 0x00, 0x33, 0xff);
  const stream = Uint8Array.from([
    ...envelope(1, avcc),
    ...envelope(2, Uint8Array.of(9, 9, 9)),
    ...envelope(9, Uint8Array.of(1)), // unknown tags are skipped
    ...envelope(3, Uint8Array.of(7)),
  ]);
  for (const step of [1, 3, 7, stream.length]) {
    const reader = new p.EnvelopeReader();
    const out = [];
    for (let i = 0; i < stream.length; i += step) out.push(...reader.push(stream.subarray(i, i + step)));
    assert.deepEqual(out.map((e) => e.kind), ['description', 'key', 'delta'], `step ${step}`);
    assert.deepEqual([...out[1].payload], [9, 9, 9]);
  }
  assert.equal(p.avcCodec(avcc), 'avc1.640033');
});

test('SEMU packets carry key flag and pts; bare packets fall back to scanning', () => {
  const body = Uint8Array.of(0, 0, 0, 1, 0x67, 0x42, 0xe0, 0x1f, 0, 0, 1, 0x65, 0x88);
  const packet = new Uint8Array(16 + body.length);
  const v = new DataView(packet.buffer);
  v.setUint32(0, 0x53454d55);
  v.setUint8(4, 1);
  v.setUint8(5, 1);
  v.setBigUint64(8, 42n);
  packet.set(body, 16);
  const read = p.readSemu(packet.buffer);
  assert.equal(read.key, true);
  assert.equal(read.pts, 42);
  assert.equal(read.data.length, body.length);
  const bare = p.readSemu(body.slice().buffer);
  assert.equal(bare.key, null);
  const scan = p.scanAnnexB(bare.data);
  assert.equal(scan.key, true);
  assert.equal(p.avcCodec(scan.sps), 'avc1.42E01F');
});

test('keys map to HID usages and touches follow rotation', () => {
  assert.equal(p.hidUsage('KeyA'), 0x04);
  assert.equal(p.hidUsage('Digit1'), 0x1e);
  assert.equal(p.hidUsage('Digit0'), 0x27);
  assert.equal(p.hidUsage('Enter'), 0x28);
  assert.equal(p.hidUsage('F13'), null);
  const portrait = { width: 1206, height: 2622 };
  assert.deepEqual(p.iosTouchPoint(0.2, 0.3, { ...portrait, orientation: 'portrait' }), { x: 0.2, y: 0.3 });
  assert.deepEqual(p.iosTouchPoint(0.2, 0.3, { ...portrait, orientation: 'landscape_left' }), { x: 0.3, y: 0.8 });
  assert.deepEqual(p.iosTouchPoint(0.25, 0.5, { ...portrait, orientation: 'portrait_upside_down' }), { x: 0.75, y: 0.5 });
  const msg = p.tagged(p.IOS_TOUCH, { type: 'begin', x: 0.5, y: 0.5 });
  assert.equal(msg[0], 0x03);
  assert.deepEqual(JSON.parse(new TextDecoder().decode(msg.subarray(1))), { type: 'begin', x: 0.5, y: 0.5 });
});
