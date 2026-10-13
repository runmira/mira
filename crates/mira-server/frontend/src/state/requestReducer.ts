import { recordAskUserDecision,recordPlanDecision,updateTool,type Entry } from '../transcript/entries';
import type { ServerMsg } from '../types';
export type Resolution = Extract<ServerMsg, { type: 'prompt_resolved' | 'approval_resolved' }>;
export function applyResolution(entries: Entry[], resolution: Resolution): Entry[] {
  if (resolution.type === 'approval_resolved') return updateTool(entries, resolution.call_id,
    tool => tool.status === 'pending' ? { ...tool, status: resolution.allow ? 'running' : 'denied' } : tool);
  if (resolution.kind === 'plan') return recordPlanDecision(entries, resolution.prompt_id,
    { approved: !!resolution.approved, steps: resolution.steps ?? undefined, note: resolution.note ?? undefined });
  if (resolution.kind === 'ask_user') return recordAskUserDecision(entries, resolution.prompt_id,
    resolution.cancelled ? { cancelled: true } : { cancelled: false, answers: resolution.answers ?? [] });
  return entries;
}
