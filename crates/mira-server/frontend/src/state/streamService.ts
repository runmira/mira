import { appendToken,applyNativeFrame,type Entry,type NativeFrame } from '../transcript/entries';
export type StreamIdentity = { session: string; generation: number };
export type StreamScheduler = {
  frame: (callback: () => void) => number;
  cancelFrame: (id: number) => void;
  timer: (callback: () => void, delay: number) => ReturnType<typeof setTimeout>;
  cancelTimer: (id: ReturnType<typeof setTimeout>) => void;
  hidden: () => boolean;
};
const browserScheduler: StreamScheduler = {
  frame: callback => requestAnimationFrame(callback), cancelFrame: id => cancelAnimationFrame(id),
  timer: (callback, delay) => setTimeout(callback, delay), cancelTimer: id => clearTimeout(id),
  hidden: () => typeof document !== 'undefined' && document.hidden,
};
/** Resources only: every scheduled flush captures its immutable attachment identity. */
export function createStreamService(
  identity: () => StreamIdentity | null,
  write: (owner: StreamIdentity, update: (entries: Entry[]) => Entry[]) => void,
  scheduler = browserScheduler,
) {
  let owner: StreamIdentity | null = null;
  let tokens = '';
  let native: { frame: NativeFrame; at: number }[] = [];
  let frame: number | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let disposed = false;
  function cancel() {
    if (frame !== null) scheduler.cancelFrame(frame);
    if (timer !== null) scheduler.cancelTimer(timer);
    frame = null; timer = null;
  }
  function flushTokens() {
    const text = tokens; tokens = '';
    const at = Date.now();
    if (owner && text && !disposed) write(owner, entries => appendToken(entries, text, at));
  }
  function flushNativeFrames() {
    const frames = native; native = [];
    if (owner && frames.length && !disposed) write(owner, entries => frames.reduce((previous, item) => applyNativeFrame(previous, item.frame, item.at), entries));
  }
  function flush() { cancel(); flushTokens(); flushNativeFrames(); }
  function capture() {
    if (disposed) return false;
    const next = identity();
    if (!next) return false;
    if (owner?.session !== next.session || owner?.generation !== next.generation) { flush(); owner = next; }
    return true;
  }
  function schedule() {
    if (frame !== null || timer !== null) return;
    if (!scheduler.hidden()) frame = scheduler.frame(flush);
    // RAF can pause after scheduling when a visible window becomes hidden.
    timer = scheduler.timer(flush, scheduler.hidden() ? 50 : 250);
  }
  return {
    queueToken(text: string) { if (capture()) { flushNativeFrames(); tokens += text; schedule(); } },
    queueNative(msg: NativeFrame) {
      if (!capture()) return;
      flushTokens(); native.push({ frame: msg, at: Date.now() });
      if (native.length >= 256) flush(); else schedule();
    },
    flushTokens: () => { cancel(); flushTokens(); if (native.length) schedule(); },
    flushNativeFrames: () => { cancel(); flushNativeFrames(); if (tokens) schedule(); },
    flush,
    dispose() { cancel(); disposed = true; tokens = ''; native = []; owner = null; },
  };
}
