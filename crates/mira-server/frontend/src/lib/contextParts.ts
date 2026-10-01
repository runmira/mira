/**
 * One palette for the parts of a context window, so the usage ring's bar
 * and the context inspector color the same part the same way — for Mira's
 * own breakdown and for an agent's (whose category ids are slugs of its own
 * names, e.g. `system_tools`).
 */
const PART_COLORS: Record<string, string> = {
  system: '#a78bfa',
  system_prompt: '#a78bfa',
  tools: '#60a5fa',
  system_tools: '#60a5fa',
  mcp_tools: '#38bdf8',
  memory: '#2dd4bf',
  memory_files: '#2dd4bf',
  skills: '#34d399',
  conversation: '#f472b6',
  messages: '#f472b6',
  tool_results: '#f59e0b',
};
const SPARE_COLORS = ['#e879f9', '#a3e635', '#fb7185', '#fbbf24', '#94a3b8'];

/** A part's color: known parts keep theirs; anything else an agent
 *  reports gets a spare, by position. */
export function partColor(id: string, index: number): string {
  return PART_COLORS[id] ?? SPARE_COLORS[index % SPARE_COLORS.length];
}
