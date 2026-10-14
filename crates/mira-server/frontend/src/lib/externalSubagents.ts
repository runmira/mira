import { agentCallToToolCall, mergeAgentCall, agentToolResult, boundAgentOutput } from './agentTools';
import { applyNativeFrame, sealAcpThought, settleTools, updateSubagent, type Entry, type SubagentStreamState } from '../transcript/entries';
import type { AcpToolCall, ExternalChildFrame, RuntimeWork } from '../types';

export const externalCallId = (id: string) => `external:${id}`;
export const childCallId = (id: string) => externalCallId(`subagent:${id}`);
const workCallId = (work: RuntimeWork) => work.subagent?.call_id ?? externalCallId(work.id);
const active = (work: RuntimeWork) => ['pending', 'running', 'waiting', 'paused'].includes(work.status);

/** Stable native child identity projected through Mira's existing AgentCard. */
export function upsertExternalWork(entries: Entry[], work: RuntimeWork): Entry[] {
  if (work.kind !== 'subagent') return entries;
  const known = entries.find(e => e.kind === 'tool' && e.externalWork?.id === work.id);
  // Keep the first public identity stable for open tabs. Native call IDs
  // arriving later become aliases, not a second card or a renamed tab.
  const id = known?.kind === 'tool' ? known.call.id : workCallId(work);
  const prior = entries.find(e => e.kind === 'tool' && e.call.id === id) ?? known;
  const alias = entries.find(e => e.kind === 'tool' && e.call.id === work.subagent?.call_id && e !== prior);
  const running = active(work) && !(prior?.kind === 'tool' && prior.status === 'complete' && !work.subagent?.reopen);
  let args: Record<string, unknown> = {};
  if (prior?.kind === 'tool') { try { args = JSON.parse(prior.call.function.arguments); } catch { /* older entry */ } }
  if (alias?.kind === 'tool') { try { args = { ...args, ...JSON.parse(alias.call.function.arguments) }; } catch { /* older entry */ } }
  const call = { id, type: 'function' as const, function: { name: 'agent', arguments: JSON.stringify({
    ...args, _external: true, type: work.subagent?.name ?? args.type ?? 'External agent',
    prompt: work.subagent?.prompt ?? work.title ?? args.prompt ?? '',
  }) } };
  const entry: Entry = { ...(alias?.kind === 'tool' ? alias : {}), ...(prior?.kind === 'tool' ? prior : {}), agentCall: prior?.kind === 'tool' ? prior.agentCall ?? (alias?.kind === 'tool' ? alias.agentCall : undefined) : undefined, externalWork: work, kind: 'tool', call, preview: null, status: running ? (work.status === 'paused' ? 'paused' : 'running') : 'complete',
    result: running ? null : { call_id: id, content: work.subagent?.result ?? (prior?.kind === 'tool' ? prior.result?.content : '') ?? '',
      is_error: active(work) && !running && prior?.kind === 'tool' ? prior.result?.is_error : work.status === 'failed' || work.status === 'cancelled' },
    startedAt: prior?.kind === 'tool' ? prior.startedAt : undefined };
  return prior ? entries.flatMap(e => e === prior ? [entry] : e === alias ? [] : [e]) : [...entries, entry];
}

export function projectExternalWork(streams: Map<string, SubagentStreamState>, work: RuntimeWork) {
  if (work.kind !== 'subagent') return streams;
  const known = [...streams].find(([, stream]) => stream.externalWork?.id === work.id)?.[0];
  const id = known ?? workCallId(work);
  const alias = work.subagent?.call_id && work.subagent.call_id !== id ? streams.get(work.subagent.call_id) : undefined;
  const next = updateSubagent(streams, id, s => {
    const entries = s.entries.length ? s.entries : alias?.entries ?? s.entries;
    return { ...s,
    externalWork: work,
    agentId: work.native_thread_id ?? s.agentId, agentName: work.subagent?.name ?? s.agentName ?? alias?.agentName,
    prompt: work.subagent?.prompt ?? work.title ?? s.prompt ?? alias?.prompt, done: !active(work) || (s.done && !work.subagent?.reopen),
    entries: !active(work) ? settleTools(sealAcpThought(entries)) : entries,
  }; });
  if (alias && work.subagent?.call_id) next.delete(work.subagent.call_id);
  return next;
}

export function applyExternalChild(streams: Map<string, SubagentStreamState>, id: string, frame: ExternalChildFrame, at = Date.now()) {
  const known = [...streams].find(([, stream]) => stream.agentId === id)?.[0];
  return updateSubagent(streams, known ?? childCallId(id), s => {
    const entries = applyNativeFrame(s.entries, frame, at);
    return { ...s, agentId: id, entries: s.done ? settleTools(sealAcpThought(entries, at)) : entries };
  });
}

export function projectAcpSubagent(streams: Map<string, SubagentStreamState>, call: AcpToolCall) {
  call = boundAgentOutput(mergeAgentCall(streams.get(call.id)?.agentCall, call));
  const mira = agentCallToToolCall(call);
  if (mira.function.name !== 'agent') return streams;
  const args = JSON.parse(mira.function.arguments);
  const result = agentToolResult(call);
  const progress = (call.content ?? []).flatMap(content => content.type === 'content' ? [content.text] : []).join('\n');
  return updateSubagent(streams, call.id, s => ({ ...s, agentCall: call, prompt: args.prompt,
    agentName: args.type, done: s.externalWork ? !active(s.externalWork) : result !== null,
    entries: (result?.content || progress) ? [{ kind: 'msg', msg: { role: 'assistant', content: result?.content || progress } }] : s.entries,
  }));
}


/** Rehydrate child streams without replaying child narration into the parent. */
export function replayExternalStreams(lines: readonly import('../types').AgentTranscriptLine[]) {
  let streams = new Map<string, SubagentStreamState>();
  for (const line of lines) {
    const frame = line.frame as import('../types').ServerMsg | undefined;
    if (frame?.type === 'runtime_work_updated') streams = projectExternalWork(streams, frame.work);
    else if (frame?.type === 'external_subagent_frame') {
      if (frame.work) streams = projectExternalWork(streams, frame.work);
      streams = applyExternalChild(streams, frame.agent_id, frame.frame, line.t);
    }
    else if (frame?.type === 'acp_tool_call' || frame?.type === 'acp_tool_call_update') streams = projectAcpSubagent(streams, frame.call);
    else if (frame?.type === 'acp_turn_end') streams = pauseParentChildStreams(streams);
  }
  return streams;
}

const scopedToParent = (work?: RuntimeWork) => work?.id.startsWith('subagent:acp:') || work?.id.startsWith('subagent:claude:');
/** ACP only promises child updates during this prompt. End the stream
 * without inventing completion for work the provider no longer exposes. */
export function pauseParentChildStreams(streams: Map<string, SubagentStreamState>, ids?: ReadonlySet<string>) {
  const next = new Map(streams);
  for (const [id, stream] of streams) if ((!ids || ids.has(id)) && scopedToParent(stream.externalWork) && !stream.done) {
    next.set(id, { ...stream, done: true, externalWork: { ...stream.externalWork!, status: 'paused' }, entries: settleTools(sealAcpThought(stream.entries)) });
  }
  return next;
}
export function pauseParentChildEntries(entries: Entry[]) {
  return entries.map(entry => entry.kind === 'tool' && scopedToParent(entry.externalWork) && active(entry.externalWork!)
    ? { ...entry, status: 'paused' as const, externalWork: { ...entry.externalWork!, status: 'paused' as const } } : entry);
}
