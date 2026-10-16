/**
 * Who a usage row was spent on, for Usage's per-provider views: Mira's own
 * model, or the external agent that ran the turns. One label and one series
 * color per provider, so the chart, the summary rows and Limits agree.
 */

export type UsageProvider = {
  id: string;
  label: string;
  /** SVG fill / CSS color: one hue per provider, stable across views. */
  color: string;
  /** The agent driver behind it, for its mark; `null` for Mira itself. */
  driver: string | null;
};

const KNOWN: Record<string, Omit<UsageProvider, 'id'>> = {
  mira: { label: 'Mira', color: '#7aa2f7', driver: null },
  claude: { label: 'Claude Code', color: '#d97757', driver: 'claude-code' },
  codex: { label: 'Codex', color: '#10a37f', driver: 'codex' },
  opencode: { label: 'OpenCode', color: '#5b9bbd', driver: 'opencode' },
  gemini: { label: 'Gemini', color: '#4f8df5', driver: 'gemini' },
  grok: { label: 'Grok', color: '#a1a1aa', driver: 'grok' },
  cursor: { label: 'Cursor', color: '#8b8b8b', driver: 'cursor' },
};

/** Driver kinds and their usage-row agent names differ for Claude Code. */
const ALIASES: Record<string, string> = { 'claude-code': 'claude', claude_code: 'claude' };

/** The order providers are listed and stacked in. */
export const PROVIDER_ORDER = Object.keys(KNOWN);

const FALLBACK_COLORS = ['#bb9af7', '#7dcfff', '#e0af68', '#9ece6a', '#f7768e', '#73daca'];

export function providerKey(agentOrDriver: string | null | undefined): string {
  const raw = (agentOrDriver ?? '').trim().toLowerCase();
  if (!raw) return 'mira';
  return ALIASES[raw] ?? raw;
}

export function usageProvider(agentOrDriver: string | null | undefined): UsageProvider {
  const id = providerKey(agentOrDriver);
  const known = KNOWN[id];
  if (known) return { id, ...known };
  let h = 5381;
  for (let i = 0; i < id.length; i++) h = (h * 33) ^ id.charCodeAt(i);
  return {
    id,
    label: id.replace(/[-_]+/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase()),
    color: FALLBACK_COLORS[Math.abs(h) % FALLBACK_COLORS.length],
    driver: id,
  };
}

/** `ids` in display order: known providers first, then the rest by name. */
export function sortProviders(ids: Iterable<string>): string[] {
  return [...new Set(ids)].sort((a, b) => {
    const ia = PROVIDER_ORDER.indexOf(a);
    const ib = PROVIDER_ORDER.indexOf(b);
    if (ia >= 0 || ib >= 0) return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib);
    return a.localeCompare(b);
  });
}
