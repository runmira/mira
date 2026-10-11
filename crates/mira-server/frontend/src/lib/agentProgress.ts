import type { Entry } from '../transcript/entries';
import type { ServerMsg } from '../types';

const promptStage: Record<string, number> = {
  prompt_preparing: 0,
  prompt_sent: 1,
  prompt_queued: 2,
  prompt_started: 3,
  prompt_completed: 4,
};

/** Live and replay must show the same progress, including out-of-order
 * receipt and acknowledgement events from the two sides of the pipe. */
export function appendAgentProgress(
  entries: Entry[],
  activity: Extract<ServerMsg, { type: 'stream_activity' }>,
): Entry[] {
  const entry: Entry = {
    kind: 'activity',
    activityKind: activity.kind,
    title: activity.title,
    detail: activity.detail,
  };
  const last = entries.at(-1);
  if (
    last?.kind === 'activity' &&
    last.activityKind in promptStage &&
    activity.kind in promptStage &&
    activity.kind !== 'prompt_preparing'
  ) {
    if (promptStage[activity.kind] < promptStage[last.activityKind]) return entries;
    return [...entries.slice(0, -1), entry];
  }
  return [...entries, entry];
}

export function agentFailureEntry(failure: Extract<ServerMsg, { type: 'acp_failure' }>): Entry {
  return {
    kind: 'error',
    text: failure.message,
    responseFailure: true,
    failureCode: failure.code,
    retryable: failure.retryable,
    resetAt: failure.reset_at,
  };
}

export function hasCurrentAgentFailure(entries: Entry[]): boolean {
  for (let index = entries.length - 1; index >= 0; index--) {
    const entry = entries[index];
    if (entry.kind === 'msg' && entry.msg.role === 'user') return false;
    if (entry.kind === 'error' && entry.failureCode) return true;
  }
  return false;
}
