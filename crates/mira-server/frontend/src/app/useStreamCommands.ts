import type { NativeFrame } from '../transcript/entries';
import type { ServerMsg } from '../types';
import type { WorkspaceRuntime } from './WorkspaceRuntime';

export function useStreamCommands(context: Pick<WorkspaceRuntime,
  'streams' | 'turnBaseRef' | 'getUsage' | 'handleMessage'>) {
  const { streams, turnBaseRef, getUsage, handleMessage } = context;
  function startTurnUsage(turn: number) {
    turnBaseRef.current = { turn, base: getUsage() ?? {
      prompt_tokens: 0, completion_tokens: 0, cached_input_tokens: 0, rounds: 0,
    } };
  }
  function onMessage(msg: ServerMsg) {
    if (['acp_text', 'acp_text_snapshot', 'acp_thought', 'acp_tool_output_delta'].includes(msg.type)) {
      streams.queueNative(msg as NativeFrame);
      return;
    }
    // handleMessage owns the transition so structural events commit with final text.
    handleMessage(msg);
  }
  return { flushTokens: streams.flushTokens,
    startTurnUsage, flushNativeFrames: streams.flushNativeFrames, onMessage };
}
