/**
 * Thin WebSocket client for `mira serve`.
 *
 * One long-lived socket per extension activation. Each chat request
 * calls `send(text)` and awaits the async iterator of `ServerMsg`s
 * until a `done` frame lands (or the caller aborts via the passed
 * cancellation token). Reconnects are on-demand: if the socket died,
 * the next `send` opens a fresh one.
 *
 * We intentionally don't try to reproduce the web UI's session
 * lifecycle here — no picker, no folder switching, no goal panel.
 * Those are the browser UI's job; the extension speaks the same
 * protocol just to stream chat responses.
 */

import WebSocket from 'ws';
import type { ClientMsg, ServerMsg } from './types';

export type SendResult = AsyncIterable<ServerMsg>;

/** Opaque token the caller passes to `send()` to abort a stream in
 *  flight — usually derived from VS Code's `CancellationToken`. */
export interface AbortSignalLike {
  aborted: boolean;
  onAbort(cb: () => void): void;
}

export class MiraClient {
  private ws: WebSocket | null = null;
  private baseUrl: string;
  private connectPromise: Promise<void> | null = null;
  /** Set to a per-request emitter while a `send` is in flight — every
   *  incoming frame gets pushed here so the async iterator sees it. */
  private currentSink: ((msg: ServerMsg) => void) | null = null;

  constructor(baseUrl: string) {
    this.baseUrl = normalizeBase(baseUrl);
  }

  /** True when a socket is up. */
  isConnected(): boolean {
    return this.ws?.readyState === WebSocket.OPEN;
  }

  /** Force a reconnect on the next send. Used by `/reset` so a fresh
   *  session id is minted server-side. */
  async reset(): Promise<void> {
    const s = this.ws;
    this.ws = null;
    this.connectPromise = null;
    if (s && s.readyState !== WebSocket.CLOSED) {
      s.close();
    }
  }

  /** Fire-and-forget control message. Reconnects if needed. */
  async control(msg: ClientMsg): Promise<void> {
    await this.ensureConnected();
    this.ws!.send(JSON.stringify(msg));
  }

  /**
   * Send a user turn and yield each incoming frame until `done` (or
   * the abort signal fires). The caller renders as they go — we don't
   * buffer.
   *
   * Only one send may be in flight at a time; concurrent calls
   * serialize on `currentSink`. That matches the server, which drives
   * one turn at a time per session.
   */
  async *send(text: string, abort: AbortSignalLike): SendResult {
    if (this.currentSink) {
      // A prior send never completed (usually because the user
      // navigated away mid-stream). Interrupt on the server so we
      // don't leak the old turn, then take over.
      try { await this.control({ type: 'interrupt' }); } catch { /* noop */ }
    }

    await this.ensureConnected();

    // Wire up a queue + resolver. The socket's on('message') handler
    // pushes into `queue` and wakes whichever pending resolver exists.
    const queue: ServerMsg[] = [];
    let waiter: ((m: ServerMsg | null) => void) | null = null;
    let closed = false;
    const push = (m: ServerMsg) => {
      if (waiter) {
        const w = waiter;
        waiter = null;
        w(m);
      } else {
        queue.push(m);
      }
    };
    const closeStream = () => {
      if (closed) return;
      closed = true;
      if (waiter) {
        const w = waiter;
        waiter = null;
        w(null);
      }
    };

    this.currentSink = push;

    // Fire the turn.
    const payload: ClientMsg = { type: 'send', text };
    this.ws!.send(JSON.stringify(payload));

    // If the user aborts (Esc in the chat pane, extension deactivate,
    // etc.), tell the server and end the stream — the model may still
    // be talking, but we're done listening.
    abort.onAbort(() => {
      try {
        this.ws?.send(JSON.stringify({ type: 'interrupt' } satisfies ClientMsg));
      } catch { /* socket may be dead */ }
      closeStream();
    });

    try {
      while (!closed) {
        // Drain buffered frames first.
        while (queue.length > 0 && !closed) {
          const m = queue.shift()!;
          yield m;
          if (m.type === 'done') { closeStream(); break; }
        }
        if (closed) break;
        // Nothing buffered — park until push() wakes us or the socket dies.
        const next = await new Promise<ServerMsg | null>((resolve) => {
          waiter = resolve;
        });
        if (next === null) break;
        yield next;
        if (next.type === 'done') { closeStream(); break; }
      }
    } finally {
      this.currentSink = null;
    }
  }

  private async ensureConnected(): Promise<void> {
    if (this.isConnected()) return;
    if (this.connectPromise) return this.connectPromise;
    this.connectPromise = new Promise((resolve, reject) => {
      const url = this.wsUrl();
      const s = new WebSocket(url);
      s.on('open', () => {
        this.ws = s;
        // The server sends `ready` immediately after upgrade — we
        // don't wait for it; the frame just gets dropped by the
        // no-sink path below (or forwarded if a send already fired).
        resolve();
      });
      s.on('message', (data) => {
        let parsed: ServerMsg | null = null;
        try {
          parsed = JSON.parse(data.toString()) as ServerMsg;
        } catch {
          // Malformed frame → drop. Keeps the loop resilient to a
          // server-side wire change we haven't taught the client yet.
          return;
        }
        if (this.currentSink) {
          this.currentSink(parsed);
        }
      });
      s.on('close', () => {
        this.ws = null;
        this.connectPromise = null;
        // Any active stream ends the next time it awaits.
        if (this.currentSink) {
          // Deliver a synthetic `done` so the async iterator terminates.
          this.currentSink({ type: 'done' });
        }
      });
      s.on('error', (err) => {
        this.ws = null;
        this.connectPromise = null;
        reject(err);
      });
    });
    try {
      await this.connectPromise;
    } finally {
      this.connectPromise = null;
    }
  }

  private wsUrl(): string {
    const base = new URL(this.baseUrl);
    base.protocol = base.protocol === 'https:' ? 'wss:' : 'ws:';
    base.pathname = base.pathname.replace(/\/+$/, '') + '/ws';
    return base.toString();
  }
}

function normalizeBase(u: string): string {
  return u.replace(/\/+$/, '');
}
