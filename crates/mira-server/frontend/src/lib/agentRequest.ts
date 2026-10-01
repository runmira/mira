/**
 * An external agent's permission request, read out of the synthetic tool
 * call the server wraps it in (`source: "native"`).
 *
 * Agent requests go through the same approval flow as Mira's own tools —
 * same composer card, same keys, same scopes — so the only thing that
 * differs is how the request is *described*: the agent's own tool name and
 * input, and its reason for asking.
 */
import type { ToolCall } from '../types';

export type AgentRequest = {
  /** The agent's tool name, e.g. `Bash`, `Edit`, `mcp__github__…`. */
  tool: string;
  input: Record<string, unknown>;
  /** Why the agent's own policy wants a human ("Contains simple_expansion"). */
  reason: string | null;
};

export function agentRequestOf(call: ToolCall): AgentRequest | null {
  let args: Record<string, unknown>;
  try {
    args = JSON.parse(call.function.arguments || '{}');
  } catch {
    return null;
  }
  if (args?.source !== 'native') return null;
  const input = args.input && typeof args.input === 'object' ? (args.input as Record<string, unknown>) : {};
  const reason = typeof args.reason === 'string' && args.reason.trim() ? args.reason.trim() : null;
  const tool = typeof args.title === 'string' && args.title ? args.title : call.function.name;
  return { tool, input, reason };
}

export function isAgentRequest(call: ToolCall): boolean {
  return agentRequestOf(call) !== null;
}

/** The one line worth showing for a request: the command, the file, the URL. */
export function agentRequestHeadline(r: AgentRequest): string | null {
  const i = r.input;
  for (const k of ['command', 'file_path', 'path', 'url', 'pattern', 'query']) {
    const v = i[k];
    if (typeof v === 'string' && v.trim()) return v.trim();
  }
  return null;
}
