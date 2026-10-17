/**
 * One live device: video onto a canvas (or an MJPEG <img> when this
 * browser can't decode the simulator's H.264 profile), plus touch, keys and
 * hardware buttons back to it. Reconnects on its own until stopped.
 */
import type { AndroidStream, IosStream, Platform } from './api';
import {
  ANDROID_KEYCODE, EnvelopeReader, IOS_BUTTON, IOS_INPUT_ADMITTED, IOS_KEY, IOS_ORIENTATION,
  IOS_SCREEN_CONFIG, IOS_TOUCH, ORIENTATIONS, avcCodec, hidUsage, iosTouchPoint, readSemu,
  scanAnnexB, tagged, type Orientation,
} from './protocol';

export type StreamStatus = 'connecting' | 'live' | 'error';
export type Screen = { width: number; height: number; orientation: Orientation };
export type Button = 'home' | 'back' | 'recents' | 'power';

export type StreamEvents = {
  onStatus(status: StreamStatus, detail?: string): void;
  /** Whether taps and keys reach the device right now. */
  onInput(ready: boolean, detail?: string): void;
  onScreen(screen: Screen): void;
  /** The MJPEG fallback is in use; show this URL in an <img>. */
  onMjpeg(url: string | null): void;
};

const RETRY_MS = 1000;
const STALL_MS = 15_000;
/** Drop to a fresh keyframe rather than fall further behind. */
const MAX_DECODE_QUEUE = 8;
const FRAME_US = 16_667;

const wsUrl = (path: string) => `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}${path}`;

export class DeviceStream {
  private stopped = true;
  /** Bumped on every start/stop so stale callbacks drop out. */
  private generation = 0;
  private socket: WebSocket | null = null;
  private video: AbortController | null = null;
  private decoder: VideoDecoder | null = null;
  private awaitingKey = true;
  private configuring = false;
  private ts = 0;
  private inputAdmitted = false;
  private inputBlocked = false;
  private screen: Screen | null = null;
  private retries = new Map<string, ReturnType<typeof setTimeout>>();

  constructor(
    private readonly platform: Platform,
    private readonly stream: IosStream | AndroidStream,
    private readonly canvas: HTMLCanvasElement,
    private readonly events: StreamEvents,
  ) {}

  start() {
    if (!this.stopped) return;
    this.stopped = false;
    this.generation++;
    this.events.onStatus('connecting');
    const webCodecs = typeof VideoDecoder !== 'undefined' && typeof EncodedVideoChunk !== 'undefined';
    if (this.platform === 'ios') {
      void this.connectIosInput();
      if (webCodecs) void this.readIosVideo();
      else this.useMjpeg();
    } else if (webCodecs) {
      this.connectAndroid();
    } else {
      this.events.onStatus('error', 'This browser can’t decode the Android stream. Open Mira over HTTPS or localhost.');
    }
  }

  stop() {
    if (this.stopped) return;
    this.stopped = true;
    this.generation++;
    for (const t of this.retries.values()) clearTimeout(t);
    this.retries.clear();
    this.video?.abort();
    this.video = null;
    const s = this.socket;
    this.socket = null;
    s?.close();
    this.closeDecoder();
    this.events.onMjpeg(null);
  }

  // ---- input ---------------------------------------------------------------

  private send(payload: Uint8Array<ArrayBuffer> | string) {
    const ready = this.platform !== 'ios' || (this.inputAdmitted && !this.inputBlocked);
    if (ready && this.socket?.readyState === WebSocket.OPEN) this.socket.send(payload);
  }

  /** `x`, `y` are 0…1 across the shown image. */
  touch(phase: 'begin' | 'move' | 'end', x: number, y: number) {
    if (this.platform === 'ios') {
      this.send(tagged(IOS_TOUCH, { type: phase, ...iosTouchPoint(x, y, this.screen) }));
    } else {
      this.send(JSON.stringify({ type: 'touch', action: phase === 'begin' ? 'down' : phase === 'move' ? 'move' : 'up', x, y }));
    }
  }

  /** Returns whether the key was sent, so the caller can preventDefault. */
  key(event: KeyboardEvent, phase: 'down' | 'up'): boolean {
    if (this.platform === 'ios') {
      const usage = hidUsage(event.code);
      if (usage === null) return false;
      this.send(tagged(IOS_KEY, { type: phase, usage }));
      return true;
    }
    if (phase !== 'down') return ANDROID_KEYCODE[event.key] !== undefined || event.key.length === 1;
    if (event.key === 'Escape') { this.send(JSON.stringify({ type: 'back' })); return true; }
    const keycode = ANDROID_KEYCODE[event.key];
    if (keycode !== undefined) { this.send(JSON.stringify({ type: 'key', keycode })); return true; }
    if (event.key.length === 1 && !event.metaKey && !event.ctrlKey) {
      this.send(JSON.stringify({ type: 'text', text: event.key }));
      return true;
    }
    return false;
  }

  button(button: Button) {
    if (this.platform === 'ios') {
      const name = button === 'home' ? 'home' : button === 'recents' ? 'app_switcher' : button === 'power' ? 'lock' : null;
      if (name) this.send(tagged(IOS_BUTTON, { button: name }));
    } else {
      this.send(JSON.stringify({ type: button }));
    }
  }

  /** iOS: turn a quarter clockwise. */
  rotate() {
    if (this.platform !== 'ios') return;
    const now = this.screen?.orientation ?? 'portrait';
    this.send(tagged(IOS_ORIENTATION, { orientation: ORIENTATIONS[(ORIENTATIONS.indexOf(now) + 1) % 4] }));
  }

  // ---- video ---------------------------------------------------------------

  private paint(frame: VideoFrame | ImageBitmap, w: number, h: number) {
    if (this.canvas.width !== w || this.canvas.height !== h) {
      this.canvas.width = w;
      this.canvas.height = h;
    }
    this.canvas.getContext('2d')?.drawImage(frame, 0, 0, w, h);
  }

  private makeDecoder() {
    const generation = this.generation;
    return new VideoDecoder({
      output: (frame) => {
        try {
          if (generation === this.generation) {
            this.paint(frame, frame.displayWidth, frame.displayHeight);
            this.events.onStatus('live');
          }
        } finally {
          frame.close();
        }
      },
      error: () => this.closeDecoder(),
    });
  }

  private closeDecoder() {
    const d = this.decoder;
    this.decoder = null;
    this.awaitingKey = true;
    this.configuring = false;
    if (d && d.state !== 'closed') { try { d.close(); } catch { /* already closed */ } }
  }

  private async configure(config: VideoDecoderConfig): Promise<boolean> {
    const full = { ...config, optimizeForLatency: true };
    const ok = await VideoDecoder.isConfigSupported(full).then((s) => !!s.supported, () => false);
    if (!ok || this.stopped) return false;
    try {
      if (!this.decoder || this.decoder.state === 'closed') this.decoder = this.makeDecoder();
      this.decoder.configure(full);
      this.awaitingKey = true;
      return true;
    } catch {
      return false;
    }
  }

  /** False when the decoder fell behind and the feed should restart. */
  private decode(key: boolean, data: Uint8Array, pts?: number | null): boolean {
    const d = this.decoder;
    if (!d || d.state !== 'configured') return true;
    if (this.awaitingKey) {
      if (!key) return true;
      this.awaitingKey = false;
    }
    if (d.decodeQueueSize > MAX_DECODE_QUEUE) return false;
    try {
      d.decode(new EncodedVideoChunk({ type: key ? 'key' : 'delta', timestamp: pts ?? this.ts, data }));
      this.ts += FRAME_US;
    } catch {
      return false;
    }
    return true;
  }

  private retry(channel: string, run: () => void) {
    if (this.stopped || this.retries.has(channel)) return;
    this.retries.set(channel, setTimeout(() => { this.retries.delete(channel); run(); }, RETRY_MS));
  }

  private useMjpeg() {
    this.closeDecoder();
    this.events.onMjpeg((this.stream as IosStream).mjpeg);
    this.events.onStatus('live');
  }

  private async readIosVideo() {
    const generation = this.generation;
    const ctl = new AbortController();
    this.video = ctl;
    const current = () => !this.stopped && this.generation === generation && this.video === ctl;
    const reader = new EnvelopeReader();
    try {
      const res = await fetch((this.stream as IosStream).video, { signal: ctl.signal, cache: 'no-store' });
      if (!res.ok || !res.body) throw new Error(`video ${res.status}`);
      const body = res.body.getReader();
      for (;;) {
        // A body can stay open after the simulator stops sending frames.
        const stall = setTimeout(() => ctl.abort(), STALL_MS);
        let chunk: ReadableStreamReadResult<Uint8Array>;
        try { chunk = await body.read(); } finally { clearTimeout(stall); }
        if (!current() || chunk.done) break;
        for (const env of reader.push(chunk.value)) {
          if (env.kind === 'seed') {
            void createImageBitmap(new Blob([env.payload as BlobPart], { type: 'image/jpeg' })).then((bmp) => {
              if (current() && !this.decoder) this.paint(bmp, bmp.width, bmp.height);
              bmp.close();
            }, () => {});
          } else if (env.kind === 'description') {
            if (!(await this.configure({ codec: avcCodec(env.payload), description: env.payload }))) {
              await body.cancel().catch(() => {});
              if (current()) this.useMjpeg();
              return;
            }
          } else if (!this.decode(env.kind === 'key', env.payload)) {
            // Behind: a new subscription starts at a fresh keyframe.
            ctl.abort();
            break;
          }
        }
      }
    } catch {
      /* aborted or dropped; retried below */
    }
    if (!current() && this.video !== ctl) return;
    this.closeDecoder();
    if (!this.stopped && this.generation === generation) {
      this.events.onStatus('connecting', 'Reconnecting…');
      this.retry('video', () => void this.readIosVideo());
    }
  }

  private async connectIosInput() {
    if (this.stopped) return;
    const generation = this.generation;
    // The input socket only attaches once the simulator's helper is up;
    // touching the MJPEG feed starts it.
    try {
      const ctl = new AbortController();
      const t = setTimeout(() => ctl.abort(), 2000);
      const res = await fetch((this.stream as IosStream).mjpeg, { signal: ctl.signal, cache: 'no-store' });
      await res.body?.getReader().read();
      clearTimeout(t);
      ctl.abort();
    } catch { /* best effort */ }
    if (this.stopped || this.generation !== generation) return;
    const ws = new WebSocket(wsUrl((this.stream as IosStream).input));
    ws.binaryType = 'arraybuffer';
    this.socket = ws;
    this.inputAdmitted = false;
    this.inputBlocked = false;
    ws.onmessage = (event) => {
      if (this.socket !== ws || !(event.data instanceof ArrayBuffer)) return;
      const bytes = new Uint8Array(event.data);
      if (bytes[0] === IOS_INPUT_ADMITTED) {
        this.inputAdmitted = true;
        this.events.onInput(!this.inputBlocked);
      } else if (bytes[0] === IOS_SCREEN_CONFIG) {
        try {
          const cfg = JSON.parse(new TextDecoder().decode(bytes.subarray(1))) as Screen & { inputUnavailable?: boolean };
          this.inputBlocked = cfg.inputUnavailable === true;
          this.screen = { width: cfg.width, height: cfg.height, orientation: cfg.orientation };
          this.events.onScreen(this.screen);
          this.events.onInput(this.inputAdmitted && !this.inputBlocked, this.inputBlocked ? 'Simulator input is unavailable' : undefined);
        } catch { /* ignore a malformed config */ }
      }
    };
    ws.onclose = () => {
      if (this.socket !== ws) return;
      this.socket = null;
      this.events.onInput(false);
      this.retry('input', () => void this.connectIosInput());
    };
    ws.onerror = () => ws.close();
  }

  private connectAndroid() {
    if (this.stopped) return;
    const ws = new WebSocket(wsUrl(`${(this.stream as AndroidStream).ws}&frame-meta=1`));
    ws.binaryType = 'arraybuffer';
    this.socket = ws;
    const resetVideo = () => { if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ type: 'reset-video', ack: false })); };
    ws.onopen = () => this.events.onInput(true);
    ws.onmessage = (event) => {
      if (this.socket !== ws) return;
      if (typeof event.data === 'string') {
        // A rotation restarts the encoder at a new size.
        if (event.data.includes('"video-session"')) { this.closeDecoder(); resetVideo(); }
        return;
      }
      const packet = readSemu(event.data as ArrayBuffer);
      const ready = this.decoder?.state === 'configured';
      const scan = packet.key === null || (packet.key && !ready) ? scanAnnexB(packet.data) : null;
      const key = packet.key ?? scan?.key ?? false;
      if (scan?.sps && !ready) {
        if (this.configuring) return;
        this.configuring = true;
        void this.configure({ codec: avcCodec(scan.sps) }).then((ok) => {
          this.configuring = false;
          if (ok) resetVideo();
          else this.events.onStatus('error', 'This browser can’t decode the emulator’s video.');
        });
        return;
      }
      if (!ready) { if (!key) resetVideo(); return; }
      if (!this.decode(key, packet.data, packet.pts)) { this.closeDecoder(); resetVideo(); }
    };
    ws.onclose = (event) => {
      if (this.socket !== ws) return;
      this.socket = null;
      this.closeDecoder();
      this.events.onInput(false, event.reason || undefined);
      this.events.onStatus('connecting', 'Reconnecting…');
      this.retry('android', () => this.connectAndroid());
    };
    ws.onerror = () => ws.close();
  }
}
