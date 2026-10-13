import { parseSentAttachments } from '../components/Composer';
import type { ActivityTurn } from '../components/panes/ActivityPane';
import type { SubagentTab } from '../components/SubagentPanel';
import type { MinimapItem } from '../components/TimelineMinimap';
import { groupByTurn } from '../lib/turnActivity';
import type { Entry,SubagentStreamState } from '../transcript/entries';
export function liveTranscriptView(entries: Entry[], offset: number) {
  const turns = groupByTurn(entries);
  const minimapItems: MinimapItem[] = [];
  const activityTurns: ActivityTurn[] = [];
  turns.forEach((turn, i) => {
    if (turn.user?.kind !== 'msg' || turn.user.msg.role !== 'user') return;
    const text = parseSentAttachments(turn.user.msg.content ?? '').text.trim();
    const id = `turn-${turn.user.transcriptTurnIndex ?? i + offset}`;
    const assistant = turn.body.find(entry => entry.kind === 'msg' && entry.msg.role === 'assistant' && entry.msg.content?.trim());
    minimapItems.push({ id, userText: text, assistantText: assistant?.kind === 'msg' ? (assistant.msg.content ?? null) : null });
    activityTurns.push({ id, userIdx: entries.indexOf(turn.user), text, body: turn.body });
  });
  return { turns, minimapItems, activityTurns,
    isEmpty: entries.every(entry => entry.kind === 'msg' && !entry.msg.content?.trim() && !entry.msg.tool_calls?.length) };
}
export function liveSubagentTabs(ids: string[], entries: Entry[], streams: Map<string, SubagentStreamState>): SubagentTab[] {
  return ids.flatMap(callId => {
    const entry = entries.find(entry => entry.kind === 'tool' && entry.call.id === callId);
    if (entry?.kind !== 'tool') return [];
    const stream = streams.get(callId);
    return [{ callId, call: entry.call, status: entry.status, result: entry.result,
      streamEntries: stream?.entries ?? [], streamDone: stream?.done ?? false, pendingReview: stream?.pendingReview ?? null }];
  });
}
