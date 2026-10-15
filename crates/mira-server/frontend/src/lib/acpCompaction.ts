import type { Entry } from '../transcript/entries';
import type { CompactionEntry } from '../components/CompactionCard';
import type { ServerMsg } from '../types';
import { boundedOutput } from './nativeStream';

type CompactionFrame = Extract<ServerMsg, { type: 'acp_compaction' | 'acp_compaction_summary' }>;
/** The first native ID fixes timeline position; later updates patch that card. */
export function applyAcpCompaction(previous: Entry[], frame: CompactionFrame, at = Date.now()): Entry[] {
  let index = previous.findIndex(entry => entry.kind === 'compact' && entry.nativeCompactionId === frame.id);
  // Adopt the server's admitted-command placeholder instead of creating two cards.
  if (index < 0 && frame.type === 'acp_compaction') index = previous.findIndex(entry => entry.kind === 'compact' && entry.state === 'running' && entry.trigger === 'agent' && !entry.nativeCompactionId);
  const existing = index >= 0 ? previous[index] as CompactionEntry : undefined;
  if (frame.type === 'acp_compaction_summary' && (!existing || existing.state !== 'running')) return previous;
  const entry: CompactionEntry = existing ? {...existing} : {kind:'compact', state:'running', trigger:'agent', summarized:null, summary:null, startedAt:at};
  entry.nativeCompactionId = frame.id;
  if (frame.type === 'acp_compaction_summary') entry.summary = boundedOutput(entry.summary ?? '', frame.text);
  else {
    // Terminal receipts cannot be reopened by a delayed start notification.
    if (frame.status === 'in_progress' && existing && existing.state !== 'running') return previous;
    entry.state = frame.status === 'in_progress' ? 'running' : frame.status === 'completed' ? 'done' : frame.status === 'cancelled' ? 'cancelled' : 'failed';
    if (entry.state !== 'running') entry.endedAt = entry.endedAt ?? at;
    if (frame.replace_summary) entry.summary = frame.summary == null ? null : boundedOutput('', frame.summary);
    if (frame.replace_error) entry.error = frame.error;
    if (!['in_progress','completed','cancelled','failed'].includes(frame.status)) entry.error = `Unsupported compaction status: ${frame.status}`;
  }
  if (index < 0) return [...previous,entry];
  return previous.map((item,i) => i === index ? entry : item);
}

/** A turn end is not proof of native compaction success when its receipt is missing. */
export function settleAgentCompactions(previous: Entry[], successful: boolean, cancelled: boolean, error: string, at = Date.now()): Entry[] {
  return previous.map(entry => entry.kind === 'compact' && entry.trigger === 'agent' && entry.state === 'running'
    ? {...entry, endedAt:at, state:cancelled ? 'cancelled' : successful && !entry.nativeCompactionId ? 'done' : 'failed', error:successful && entry.nativeCompactionId ? 'The agent ended without reporting a compaction result.' : successful || cancelled ? null : error}
    : entry);
}
