/**
 * Wire formats of the device hub's streams, kept free of DOM and sockets so
 * they can be tested on their own.
 *
 * iOS video is an HTTP body of length-prefixed envelopes (AVCC framing):
 * `u32 length | u8 tag | payload`, where tag 1 is the decoder description
 * (avcC), 2 a keyframe, 3 a delta frame and 4 a JPEG to show until the
 * first frame decodes. iOS input is a binary WebSocket of `u8 tag | JSON`
 * messages.
 *
 * Android video and input share one WebSocket: binary "SEMU" packets
 * (`u32 magic | u8 version | u8 flags | u16 · | u64 pts` then Annex B
 * H.264) and JSON text messages for gestures and keys.
 */

export type Orientation = 'portrait' | 'landscape_left' | 'portrait_upside_down' | 'landscape_right';

export const IOS_TOUCH = 0x03;
export const IOS_BUTTON = 0x04;
export const IOS_KEY = 0x06;
export const IOS_ORIENTATION = 0x07;
/** Hub → client: screen size and orientation changed. */
export const IOS_SCREEN_CONFIG = 0x82;
/** Hub → client: this socket may send input. */
export const IOS_INPUT_ADMITTED = 0x83;

const encoder = new TextEncoder();

export function tagged(tag: number, payload: unknown): Uint8Array<ArrayBuffer> {
  const json = encoder.encode(JSON.stringify(payload));
  const out = new Uint8Array(1 + json.length);
  out[0] = tag;
  out.set(json, 1);
  return out;
}

/** `avc1.PPCCLL` from an avcC record or an SPS NAL (profile, constraints, level). */
export function avcCodec(bytes: Uint8Array): string {
  if (bytes.length < 4) return 'avc1.42E01E';
  const hex = (b: number) => b.toString(16).padStart(2, '0').toUpperCase();
  return `avc1.${hex(bytes[1])}${hex(bytes[2])}${hex(bytes[3])}`;
}

export type Envelope = { kind: 'description' | 'key' | 'delta' | 'seed'; payload: Uint8Array };

const ENVELOPE_KINDS: Record<number, Envelope['kind'] | undefined> = { 1: 'description', 2: 'key', 3: 'delta', 4: 'seed' };

/** Reassembles envelopes from arbitrary network chunks. */
export class EnvelopeReader {
  private buf = new Uint8Array(1 << 16);
  private len = 0;

  push(chunk: Uint8Array): Envelope[] {
    if (this.len + chunk.length > this.buf.length) {
      let size = this.buf.length;
      while (size < this.len + chunk.length) size *= 2;
      const grown = new Uint8Array(size);
      grown.set(this.buf.subarray(0, this.len));
      this.buf = grown;
    }
    this.buf.set(chunk, this.len);
    this.len += chunk.length;
    const out: Envelope[] = [];
    let at = 0;
    while (this.len - at >= 4) {
      const n = new DataView(this.buf.buffer, at, 4).getUint32(0);
      if (this.len - at - 4 < n) break;
      const kind = n >= 1 ? ENVELOPE_KINDS[this.buf[at + 4]] : undefined;
      if (kind) out.push({ kind, payload: this.buf.slice(at + 5, at + 4 + n) });
      at += 4 + n;
    }
    if (at) {
      this.buf.copyWithin(0, at, this.len);
      this.len -= at;
    }
    return out;
  }
}

const SEMU_MAGIC = 0x53454d55; // "SEMU"
const SEMU_HEADER = 16;

export type SemuPacket = { data: Uint8Array; key: boolean | null; pts: number | null };

export function readSemu(raw: ArrayBuffer): SemuPacket {
  const bytes = new Uint8Array(raw);
  if (bytes.length > SEMU_HEADER) {
    const v = new DataView(raw, 0, SEMU_HEADER);
    if (v.getUint32(0) === SEMU_MAGIC && v.getUint8(4) === 1) {
      const pts = v.getBigUint64(8);
      return {
        data: bytes.subarray(SEMU_HEADER),
        key: (v.getUint8(5) & 1) === 1,
        pts: pts <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(pts) : null,
      };
    }
  }
  return { data: bytes, key: null, pts: null };
}

/** Whether an Annex B access unit holds an IDR slice, and its SPS if any. */
export function scanAnnexB(buf: Uint8Array): { key: boolean; sps: Uint8Array | null } {
  let key = false;
  let sps: Uint8Array | null = null;
  for (let i = 0; i + 2 < buf.length; ) {
    if (buf[i] === 0 && buf[i + 1] === 0) {
      const start = buf[i + 2] === 1 ? 3 : buf[i + 2] === 0 && buf[i + 3] === 1 ? 4 : 0;
      if (start) {
        const type = buf[i + start] & 0x1f;
        if (type === 7 && !sps) sps = buf.subarray(i + start);
        if (type === 5) key = true;
        i += start + 1;
        continue;
      }
    }
    i++;
  }
  return { key, sps };
}

/** USB HID usage for a `KeyboardEvent.code`, which iOS input takes. */
export function hidUsage(code: string): number | null {
  if (/^Key[A-Z]$/.test(code)) return 0x04 + code.charCodeAt(3) - 65;
  if (/^Digit[1-9]$/.test(code)) return 0x1e + code.charCodeAt(5) - 49;
  return HID[code] ?? null;
}

const HID: Record<string, number> = {
  Digit0: 0x27, Enter: 0x28, Escape: 0x29, Backspace: 0x2a, Tab: 0x2b, Space: 0x2c,
  Minus: 0x2d, Equal: 0x2e, BracketLeft: 0x2f, BracketRight: 0x30, Backslash: 0x31,
  Semicolon: 0x33, Quote: 0x34, Backquote: 0x35, Comma: 0x36, Period: 0x37, Slash: 0x38,
  Delete: 0x4c, ArrowRight: 0x4f, ArrowLeft: 0x50, ArrowDown: 0x51, ArrowUp: 0x52,
  ControlLeft: 0xe0, ShiftLeft: 0xe1, AltLeft: 0xe2, MetaLeft: 0xe3,
  ControlRight: 0xe4, ShiftRight: 0xe5, AltRight: 0xe6, MetaRight: 0xe7,
};

/** Android key codes for keys that aren't text. */
export const ANDROID_KEYCODE: Record<string, number> = {
  ArrowUp: 19, ArrowDown: 20, ArrowLeft: 21, ArrowRight: 22, Tab: 61, Enter: 66,
  Backspace: 67, Delete: 112, Home: 122, End: 123, PageUp: 92, PageDown: 93,
};

export const ORIENTATIONS: Orientation[] = ['portrait', 'landscape_left', 'portrait_upside_down', 'landscape_right'];

/**
 * The iOS stream is the raw (portrait) framebuffer even when the device is
 * turned, so a point on the shown image is mapped back into it.
 */
export function iosTouchPoint(x: number, y: number, screen: { width: number; height: number; orientation: Orientation } | null) {
  if (!screen || screen.width > screen.height) return { x, y };
  switch (screen.orientation) {
    case 'landscape_left': return { x: y, y: 1 - x };
    case 'landscape_right': return { x: 1 - y, y: x };
    case 'portrait_upside_down': return { x: 1 - x, y: 1 - y };
    default: return { x, y };
  }
}
