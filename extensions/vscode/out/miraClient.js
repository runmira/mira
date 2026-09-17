"use strict";
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
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.MiraClient = void 0;
const ws_1 = __importDefault(require("ws"));
class MiraClient {
    ws = null;
    baseUrl;
    connectPromise = null;
    /** Set to a per-request emitter while a `send` is in flight — every
     *  incoming frame gets pushed here so the async iterator sees it. */
    currentSink = null;
    constructor(baseUrl) {
        this.baseUrl = normalizeBase(baseUrl);
    }
    /** True when a socket is up. */
    isConnected() {
        return this.ws?.readyState === ws_1.default.OPEN;
    }
    /** Force a reconnect on the next send. Used by `/reset` so a fresh
     *  session id is minted server-side. */
    async reset() {
        const s = this.ws;
        this.ws = null;
        this.connectPromise = null;
        if (s && s.readyState !== ws_1.default.CLOSED) {
            s.close();
        }
    }
    /** Fire-and-forget control message. Reconnects if needed. */
    async control(msg) {
        await this.ensureConnected();
        this.ws.send(JSON.stringify(msg));
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
    async *send(text, abort) {
        if (this.currentSink) {
            // A prior send never completed (usually because the user
            // navigated away mid-stream). Interrupt on the server so we
            // don't leak the old turn, then take over.
            try {
                await this.control({ type: 'interrupt' });
            }
            catch { /* noop */ }
        }
        await this.ensureConnected();
        // Wire up a queue + resolver. The socket's on('message') handler
        // pushes into `queue` and wakes whichever pending resolver exists.
        const queue = [];
        let waiter = null;
        let closed = false;
        const push = (m) => {
            if (waiter) {
                const w = waiter;
                waiter = null;
                w(m);
            }
            else {
                queue.push(m);
            }
        };
        const closeStream = () => {
            if (closed)
                return;
            closed = true;
            if (waiter) {
                const w = waiter;
                waiter = null;
                w(null);
            }
        };
        this.currentSink = push;
        // Fire the turn.
        const payload = { type: 'send', text };
        this.ws.send(JSON.stringify(payload));
        // If the user aborts (Esc in the chat pane, extension deactivate,
        // etc.), tell the server and end the stream — the model may still
        // be talking, but we're done listening.
        abort.onAbort(() => {
            try {
                this.ws?.send(JSON.stringify({ type: 'interrupt' }));
            }
            catch { /* socket may be dead */ }
            closeStream();
        });
        try {
            while (!closed) {
                // Drain buffered frames first.
                while (queue.length > 0 && !closed) {
                    const m = queue.shift();
                    yield m;
                    if (m.type === 'done') {
                        closeStream();
                        break;
                    }
                }
                if (closed)
                    break;
                // Nothing buffered — park until push() wakes us or the socket dies.
                const next = await new Promise((resolve) => {
                    waiter = resolve;
                });
                if (next === null)
                    break;
                yield next;
                if (next.type === 'done') {
                    closeStream();
                    break;
                }
            }
        }
        finally {
            this.currentSink = null;
        }
    }
    async ensureConnected() {
        if (this.isConnected())
            return;
        if (this.connectPromise)
            return this.connectPromise;
        this.connectPromise = new Promise((resolve, reject) => {
            const url = this.wsUrl();
            const s = new ws_1.default(url);
            s.on('open', () => {
                this.ws = s;
                // The server sends `ready` immediately after upgrade — we
                // don't wait for it; the frame just gets dropped by the
                // no-sink path below (or forwarded if a send already fired).
                resolve();
            });
            s.on('message', (data) => {
                let parsed = null;
                try {
                    parsed = JSON.parse(data.toString());
                }
                catch {
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
        }
        finally {
            this.connectPromise = null;
        }
    }
    wsUrl() {
        const base = new URL(this.baseUrl);
        base.protocol = base.protocol === 'https:' ? 'wss:' : 'ws:';
        base.pathname = base.pathname.replace(/\/+$/, '') + '/ws';
        return base.toString();
    }
}
exports.MiraClient = MiraClient;
function normalizeBase(u) {
    return u.replace(/\/+$/, '');
}
//# sourceMappingURL=miraClient.js.map