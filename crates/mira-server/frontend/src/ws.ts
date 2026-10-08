import type { BackgroundMode, ClientMsg, ServerMsg } from './types';

export type WsStatus = 'connecting' | 'open' | 'closed';

export interface WsClient {
  send(msg: ClientMsg): void;
  /** Conditional mutations must not enter the reconnect replay buffer. */
  sendImmediate(msg: ClientMsg): boolean;
  /** Convenience: swap which session this WS is watching. Server updates
   *  its per-connection attached slot, replays a fresh Ready, and points
   *  the process-wide `active` pointer here so subsequent HTTP calls
   *  target the same session. */
  attach(sessionId: string): void;
  /** Set the currently-attached session's background mode. */
  setBackgroundMode(mode: BackgroundMode): void;
  /** The session the UI is showing (from its Ready frame). A reconnect
   *  re-attaches to it: a restarted server would otherwise put this socket
   *  on its own default session, and the user would silently land in a
   *  different chat. */
  setSession(sessionId: string): void;
  close(): void;
}

/**
 * Connect to `/ws` and reconnect on drop with capped exponential backoff.
 * `close()` marks the client as stopped so we don't fight a manual teardown.
 */
export function connect(
  onMsg: (msg: ServerMsg) => void,
  onStatus: (s: WsStatus) => void,
): WsClient {
  const url = (location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/ws';
  let ws: WebSocket | null = null;
  let stopped = false;
  let attempt = 0;
  let opened = false;
  let session: string | null = null;
  /** Re-attaching after a reconnect: until this session's Ready arrives,
   *  frames are the server's default session's, not the user's chat. */
  let reattaching: string | null = null;
  const queue: ClientMsg[] = [];

  function schedule() {
    if (stopped) return;
    const delay = Math.min(1000 * 2 ** attempt, 8000);
    attempt += 1;
    setTimeout(open, delay);
  }

  function open() {
    if (stopped) return;
    onStatus('connecting');
    ws = new WebSocket(url);
    const socket = ws;
    socket.onopen = () => {
      attempt = 0;
      onStatus('open');
      // On reconnect, get back onto the chat the user is looking at first —
      // attaching replays its Ready — so nothing below lands elsewhere.
      // Otherwise ask for the current state, in case a frame was missed.
      const reattach = opened && session;
      opened = true;
      reattaching = reattach ? session : null;
      socket.send(
        JSON.stringify(
          (reattach ? { type: 'attach', session_id: session } : { type: 'sync' }) as ClientMsg,
        ),
      );
      // Then what was sent while disconnected, in order.
      while (queue.length > 0 && socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify(queue.shift()!));
      }
    };
    ws.onclose = () => {
      onStatus('closed');
      ws = null;
      schedule();
    };
    ws.onerror = () => {
      // onclose fires next; let it drive the reconnect.
    };
    ws.onmessage = (ev) => {
      let msg: ServerMsg;
      try {
        msg = JSON.parse(ev.data) as ServerMsg;
      } catch (e) {
        console.error('bad frame', ev.data, e);
        return;
      }
      if (reattaching) {
        // A restarted server greets the socket with its own default
        // session; showing that, even briefly, flashes another chat.
        if (msg.type === 'session_activity' || msg.type === 'session_activity_snapshot') {
          onMsg(msg); return;
        }
        if (msg.type !== 'ready' || msg.session_id !== reattaching) return;
        reattaching = null;
      }
      onMsg(msg);
    };
  }

  const resync = () => { if (!document.hidden && ws?.readyState === WebSocket.OPEN) sendMsg({type:'sync_activity'}); };
  document.addEventListener('visibilitychange',resync);
  window.addEventListener('online',resync);
  open();

  function sendMsg(msg: ClientMsg) {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(msg));
    } else {
      // Queue so a submit made during a brief drop still lands.
      queue.push(msg);
    }
  }

  return {
    send: sendMsg,
    sendImmediate(msg) {
      if (!ws || ws.readyState !== WebSocket.OPEN || reattaching) return false;
      ws.send(JSON.stringify(msg));
      return true;
    },
    attach(sessionId) {
      sendMsg({ type: 'attach', session_id: sessionId });
    },
    setBackgroundMode(mode) {
      sendMsg({ type: 'set_background_mode', mode });
    },
    setSession(sessionId) {
      session = sessionId || null;
    },
    close() {
      stopped = true;
      document.removeEventListener('visibilitychange',resync);
      window.removeEventListener('online',resync);
      if (ws) ws.close();
    },
  };
}
