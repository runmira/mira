/** The AgentTool prefixes every tool_result with `[mira-agent-id:XYZ]\n`
 *  so the frontend can look up the child's persisted session on reload.
 *  Kept as a plain marker rather than a JSON blob so it's readable if a
 *  model does see it and grep-able for debugging. */
export const AGENT_ID_MARKER = /^\[mira-agent-id:([^\]\n]+)\]\n?/;

/** Pull the child session id out of a tool_result content. Returns null
 *  if the marker isn't present (e.g. tool errored out before the child
 *  session existed). */
export function extractAgentId(content: string | null | undefined): string | null {
  if (!content) return null;
  const m = AGENT_ID_MARKER.exec(content);
  return m ? m[1] : null;
}

/** Strip the marker from a tool_result content so it never leaks into
 *  the UI. Safe on strings without the marker — returns unchanged. */
export function stripAgentIdMarker(content: string): string {
  return content.replace(AGENT_ID_MARKER, '');
}
