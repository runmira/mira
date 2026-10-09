import { appendToken, applyNativeFrame, type NativeFrame } from '../transcript/entries';
import type { ServerMsg } from '../types';

import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function useStreamCommands(
  context: Pick<
    WorkspaceRuntime,
    | 'tokenRafRef'
    | 'tokenBufRef'
    | 'setEntries'
    | 'turnBaseRef'
    | 'usageRef'
    | 'nativeRafRef'
    | 'nativeFramesRef'
    | 'handleMessage'
  >,
) {
  const {
    tokenRafRef,
    tokenBufRef,
    setEntries,
    turnBaseRef,
    usageRef,
    nativeRafRef,
    nativeFramesRef,
    handleMessage,
  } = context;

  function drainTokens() {
    tokenRafRef.current = null;
    const buf = tokenBufRef.current;
    if (!buf) return;
    const n = Math.max(3, Math.ceil(buf.length / 6));
    const chunk = buf.slice(0, n);
    tokenBufRef.current = buf.slice(n);
    setEntries((prev) => appendToken(prev, chunk));
    if (tokenBufRef.current) tokenRafRef.current = requestAnimationFrame(drainTokens);
  }

  /** Release everything buffered now — before any other frame lands, so
   *  ordering between text and tool calls is preserved. */
  function flushTokens() {
    if (tokenRafRef.current != null) cancelAnimationFrame(tokenRafRef.current);
    tokenRafRef.current = null;
    const buf = tokenBufRef.current;
    tokenBufRef.current = '';
    if (buf) setEntries((prev) => appendToken(prev, buf));
  }

  function startTurnUsage(turn: number) {
    turnBaseRef.current = {
      turn,
      base: usageRef.current ?? {
        prompt_tokens: 0,
        completion_tokens: 0,
        cached_input_tokens: 0,
        rounds: 0,
      },
    };
  }

  function flushNativeFrames() {
    if (nativeRafRef.current != null) cancelAnimationFrame(nativeRafRef.current);
    nativeRafRef.current = null;
    const frames = nativeFramesRef.current;
    nativeFramesRef.current = [];
    if (frames.length) setEntries((entries) => frames.reduce(applyNativeFrame, entries));
  }

  function onMessage(msg: ServerMsg) {
    if (
      [
        'acp_text',
        'acp_text_snapshot',
        'acp_thought',
        'acp_tool_call',
        'acp_tool_call_update',
        'acp_tool_output_delta',
      ].includes(msg.type)
    ) {
      flushTokens();
      nativeFramesRef.current.push(msg as NativeFrame);
      // Hidden tabs throttle RAF: bound the backlog and keep replay current.
      if (document.hidden || nativeFramesRef.current.length >= 256) flushNativeFrames();
      else if (nativeRafRef.current == null)
        nativeRafRef.current = requestAnimationFrame(flushNativeFrames);
      return;
    }
    flushNativeFrames();
    handleMessage(msg);
  }
  return { drainTokens, flushTokens, startTurnUsage, flushNativeFrames, onMessage };
}
