import { agentCallToToolCall, agentToolResult, boundAgentOutput } from './agentTools';
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
  const id = work.subagent?.call_id ?? (known?.kind === 'tool' ? known.call.id : workCallId(work));
  const prior = entries.find(e => e.kind === 'tool' && e.call.id === id);
  const running = active(work) && !(prior?.kind === 'tool' && prior.status === 'complete' && !work.subagent?.reopen);
  let args: Record<string, unknown> = {};
  if (prior?.kind === 'tool') { try { args = JSON.parse(prior.call.function.arguments); } catch { /* older entry */ } }
  const call = { id, type: 'function' as const, function: { name: 'agent', arguments: JSON.stringify({
    ...args, _external: true, type: work.subagent?.name ?? args.type ?? 'External agent',
    prompt: work.subagent?.prompt ?? work.title ?? args.prompt ?? '',
  }) } };
  const entry: Entry = { ...(prior?.kind === 'tool' ? prior : {}), externalWork: work, kind: 'tool', call, preview: null, status: running ? 'running' : 'complete',
    result: running ? null : { call_id: id, content: work.subagent?.result ?? (prior?.kind === 'tool' ? prior.result?.content : '') ?? '',
      is_error: active(work) && !running && prior?.kind === 'tool' ? prior.result?.is_error : work.status === 'failed' || work.status === 'cancelled' },
    startedAt: prior?.kind === 'tool' ? prior.startedAt : undefined };
  return prior ? entries.map(e => e === prior ? entry : e) : [...entries, entry];
}

export function projectExternalWork(streams: Map<string, SubagentStreamState>, work: RuntimeWork) {
  if (work.kind !== 'subagent') return streams;
  const known = [...streams].find(([, stream]) => stream.externalWork?.id === work.id)?.[0];
  return updateSubagent(streams, work.subagent?.call_id ?? known ?? workCallId(work), s => ({ ...s,
    externalWork: work,
    agentId: work.native_thread_id ?? s.agentId, agentName: work.subagent?.name ?? s.agentName,
    prompt: work.subagent?.prompt ?? work.title ?? s.prompt, done: !active(work) || (s.done && !work.subagent?.reopen),
    entries: !active(work) ? settleTools(sealAcpThought(s.entries)) : s.entries,
  }));
}

export function applyExternalChild(streams: Map<string, SubagentStreamState>, id: string, frame: ExternalChildFrame, at = Date.now()) {
  return updateSubagent(streams, childCallId(id), s => {
    const entries = applyNativeFrame(s.entries, frame, at);
    return { ...s, agentId: id, entries: s.done ? settleTools(sealAcpThought(entries, at)) : entries };
  });
}

export function projectAcpSubagent(streams: Map<string, SubagentStreamState>, call: AcpToolCall) {
  call = boundAgentOutput(call);
  const mira = agentCallToToolCall(call);
  if (mira.function.name !== 'agent') return streams;
  const args = JSON.parse(mira.function.arguments);
  const result = agentToolResult(call);
  const progress = (call.content ?? []).flatMap(content => content.type === 'content' ? [content.text] : []).join('\n');
  return updateSubagent(streams, call.id, s => ({ ...s, prompt: args.prompt,
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
    else if (frame?.type === 'external_subagent_frame') streams = applyExternalChild(streams, frame.agent_id, frame.frame, line.t);
    else if (frame?.type === 'acp_tool_call' || frame?.type === 'acp_tool_call_update') streams = projectAcpSubagent(streams, frame.call);
  }
  return streams;
}
