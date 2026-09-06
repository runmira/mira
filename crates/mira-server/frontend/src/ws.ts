import type { ClientMsg, ServerMsg } from './types';

export type WsStatus = 'connecting' | 'open' | 'closed';

export interface WsClient {
  send(msg: ClientMsg): void;
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
      // Drain anything queued while disconnected.
      while (queue.length > 0 && socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify(queue.shift()!));
      }
      // On reconnect, ask the server to re-broadcast current state so the
      // UI stays consistent if we missed a Ready/model_changed/etc.
      socket.send(JSON.stringify({ type: 'sync' } as ClientMsg));
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
      try {
        onMsg(JSON.parse(ev.data) as ServerMsg);
      } catch (e) {
        console.error('bad frame', ev.data, e);
      }
    };
  }

  open();

  return {
    send(msg) {
      if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify(msg));
      } else {
        // Queue so a submit made during a brief drop still lands.
        queue.push(msg);
      }
    },
    close() {
      stopped = true;
      if (ws) ws.close();
    },
  };
}
