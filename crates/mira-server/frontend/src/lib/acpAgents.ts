/**
 * Presentation data for the external-agent list.
 *
 * Kept apart from the component so the favicon mapping and the config
 * parsing are testable without rendering anything, and so adding an agent
 * later is a data change rather than a UI change. The health description
 * and the instance-config storage live here too, because two surfaces now
 * read them: Settings → Agents (full config) and the sidebar (select).
 * The config itself lives on the server (`/api/engines/:id/settings`).
 */
import type { AcpAgentStatus, AcpConfigOption, AcpSessionMode } from '../types';
import { CLAUDE_MARK } from './models';
import type { AgentSettings, AgentSettingsPatch } from '../api';

/** What older builds kept per agent in `localStorage`, API key included
 *  (#79). Only read now, to move it to the server once. */
type LegacyInstanceConfig = {
  displayName?: string;
  binaryPath?: string;
  homePath?: string;
  launchArgs?: string;
  env?: string;
  apiKey?: string;
  enabled?: boolean;
  effort?: string;
  settingSources?: string;
};

const LEGACY_STORAGE_KEY = 'mira.acp.instances.v1';

/**
 * Move agent setups saved by older builds out of `localStorage` and into
 * the server's mira.yaml, then delete them from the browser.
 *
 * Each agent is removed from the blob only once the server has it, so a
 * failed save (server down, bad config file) is retried on the next load
 * instead of losing the key. Fields already set on the server win: this
 * fills gaps, it never overwrites. Returns the agents it moved.
 */
export async function migrateLegacyAgentConfigs(
  put: (instance: string, patch: AgentSettingsPatch) => Promise<unknown>,
  get: (instance: string) => Promise<AgentSettings>,
): Promise<string[]> {
  let all: Record<string, LegacyInstanceConfig>;
  try {
    const raw = localStorage.getItem(LEGACY_STORAGE_KEY);
    if (!raw) return [];
    all = JSON.parse(raw) as Record<string, LegacyInstanceConfig>;
  } catch {
    // Unreadable: nothing recoverable in it, and it may hold a key.
    try {
      localStorage.removeItem(LEGACY_STORAGE_KEY);
    } catch {
      /* storage blocked */
    }
    return [];
  }
  const moved: string[] = [];
  for (const [instance, cfg] of Object.entries(all)) {
    try {
      const current = await get(instance);
      const patch = legacyPatch(cfg, current);
      if (Object.keys(patch).length > 0) await put(instance, patch);
      delete all[instance];
      moved.push(instance);
    } catch {
      // Unknown agent on this build: nothing to move it to, so drop it
      // rather than keep a key in the browser forever.
      if (await isUnknown(get, instance)) delete all[instance];
    }
  }
  try {
    if (Object.keys(all).length === 0) localStorage.removeItem(LEGACY_STORAGE_KEY);
    else localStorage.setItem(LEGACY_STORAGE_KEY, JSON.stringify(all));
  } catch {
    /* storage blocked */
  }
  return moved;
}

async function isUnknown(get: (instance: string) => Promise<AgentSettings>, instance: string): Promise<boolean> {
  try {
    await get(instance);
    return false;
  } catch (e) {
    return /unknown engine instance/.test(String(e));
  }
}

/** The fields of an old browser-side setup that the server doesn't have yet. */
export function legacyPatch(cfg: LegacyInstanceConfig, current: AgentSettings): AgentSettingsPatch {
  const patch: AgentSettingsPatch = {};
  const fill = <K extends keyof AgentSettingsPatch>(key: K, value: AgentSettingsPatch[K] | undefined, has: boolean) => {
    if (value !== undefined && value !== '' && !has) patch[key] = value;
  };
  fill('display_name', cfg.displayName?.trim(), !!current.display_name);
  fill('binary_path', cfg.binaryPath?.trim(), !!current.binary_path);
  fill('home_path', cfg.homePath?.trim(), !!current.home_path);
  fill('effort', cfg.effort, !!current.effort);
  fill('setting_sources', cfg.settingSources?.trim(), !!current.setting_sources);
  fill('api_key', cfg.apiKey?.trim(), current.has_api_key || !!current.api_key_env);
  const args = parseArgs(cfg.launchArgs ?? '');
  if (args.length > 0 && current.launch_args.length === 0) patch.launch_args = args;
  const env = parseEnv(cfg.env ?? '');
  const missing = Object.keys(env).filter((k) => !current.env.some((e) => e.key === k));
  if (missing.length > 0) {
    // Send the whole set: stored ones as `null` (keep), new ones in full.
    patch.env = Object.fromEntries([
      ...current.env.map((e) => [e.key, null] as const),
      ...missing.map((k) => [k, env[k]] as const),
    ]);
  }
  if (cfg.enabled === false && current.enabled) patch.enabled = false;
  return patch;
}

/** Launch args as one editable line that reads back through `parseArgs`
 *  unchanged: args with spaces are quoted (with whichever quote they
 *  don't contain, since `parseArgs` has no escapes). */
export function argsToText(args: string[]): string {
  return args
    .map((a) => (a === '' || /\s/.test(a) || /^['"]/.test(a) ? (a.includes('"') ? `'${a}'` : `"${a}"`) : a))
    .join(' ');
}

export type AgentHealthTone = 'ok' | 'warn' | 'error';

/**
 * Health only. Whether the user has switched the agent on is applied
 * separately, in the row: the two used to be conflated and every agent read
 * "Disabled" regardless of what was actually installed.
 */
export function describeAgentStatus(agent: AcpAgentStatus): {
  text: string;
  tone: AgentHealthTone;
} {
  switch (agent.state.state) {
    case 'not_found':
      // An adapter-based agent needs two installs. Reporting a bare "Not
      // installed" told someone who uses Claude Code daily that Claude Code
      // was not installed — technically true of the *adapter*, and useless.
      if (agent.cli_installed) {
        return {
          text: `Adapter not installed${agent.cli_version ? ` · agent v${agent.cli_version}` : ''}`,
          tone: 'error',
        };
      }
      return { text: 'Not installed', tone: 'error' };
    case 'failed':
      return { text: 'Failed to start', tone: 'error' };
    case 'ready': {
      const bits: string[] = [];
      if (agent.auth) bits.push(agent.auth);
      if (agent.version) bits.push(`v${agent.version}`);
      // Name the transport. "Ready" alone does not say whether Mira is
      // driving the agent directly or through an adapter, and those are
      // very different setups for the user to be looking at.
      if (agent.transport === 'native') bits.push('direct');
      return { text: bits.join(' · ') || 'Ready', tone: 'ok' };
    }
  }
}

/**
 * Favicon per agent: the vendor's own icon first, so brand colour survives.
 *
 * The first attempt used Google's s2 favicon service, matching the
 * convention in `lib/editors.ts`. That was wrong here: s2 returns whatever
 * the site's favicon looks like on a *light* page, and most of these logos
 * are near-white glyphs on a dark or transparent ground. Rendered on this
 * panel's dark tile they came out as blank white squares — every agent
 * looked identical and none was recognisable. (Google's own favicon was the
 * exception, which is why Antigravity was the only one that looked right.)
 *
 * So the vendor's real icon is used directly, with s2 kept as a fallback for
 * an agent we have not mapped. `null` means neither, and the UI falls back to
 * a monogram.
 */
const AGENT_FAVICON: Record<string, string> = {
  // Bundled: claude.ai refuses hotlinked favicons (see lib/models.ts).
  'claude-code': CLAUDE_MARK,
  claude: CLAUDE_MARK,
  codex: 'https://chatgpt.com/favicon.ico',
  cursor: 'https://cursor.com/favicon.ico',
  grok: 'https://x.ai/favicon.ico',
  opencode: 'https://opencode.ai/favicon.ico',
  antigravity: 'https://antigravity.google/favicon.ico',
};

/** Domains, kept for the s2 fallback. */
const AGENT_FAVICON_DOMAIN: Record<string, string> = {
  'claude-code': 'claude.ai',
  claude: 'claude.ai',
  codex: 'chatgpt.com',
  cursor: 'cursor.com',
  grok: 'x.ai',
  opencode: 'opencode.ai',
  antigravity: 'antigravity.google',
};

/**
 * The icon URL to try first.
 *
 * Returns a list rather than a single value: the vendor icon is preferred
 * and the s2 service is the fallback, so one dead URL does not leave an
 * agent with no icon at all.
 */
export function agentFaviconUrls(kind: string): string[] {
  const out: string[] = [];

  const direct = AGENT_FAVICON[kind];
  if (direct) out.push(direct);

  const domain = AGENT_FAVICON_DOMAIN[kind];
  if (domain) {
    out.push(`https://www.google.com/s2/favicons?domain=${domain}&sz=64`);
  }

  return out;
}

/**
 * Up to two letters for the monogram fallback.
 *
 * Prefers the first letter of each capitalised word, so "Claude Code" reads
 * as `CC` rather than `C`.
 */
export function agentMonogram(displayName: string): string {
  const words = displayName.split(/[\s_-]+/).filter(Boolean);
  if (words.length === 0) return '?';
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return (words[0][0] + words[1][0]).toUpperCase();
}

/** A `KEY=value` block → a record. Blank lines and `#` comments ignored. */
export function parseEnv(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of text.split('\n')) {
    const t = line.trim();
    if (!t || t.startsWith('#')) continue;
    const eq = t.indexOf('=');
    if (eq <= 0) continue;
    const key = t.slice(0, eq).trim();
    if (!key) continue;
    out[key] = t.slice(eq + 1).trim();
  }
  return out;
}

/**
 * Split a launch-argument string, honouring single and double quotes.
 *
 * An agent's flags routinely contain spaces (`--model "gpt 5"`), so a plain
 * `split(' ')` would silently corrupt them.
 */
export function parseArgs(text: string): string[] {
  const out: string[] = [];
  const re = /"([^"]*)"|'([^']*)'|(\S+)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    const v = m[1] ?? m[2] ?? m[3];
    if (v) out.push(v);
  }
  return out;
}

/**
 * What an agent last advertised — its model list and modes — per driver.
 *
 * An agent only reports these once it is running, so without a memory the
 * picker could not list Claude Code's models until after the first
 * message, and a fresh chat inherited from an agent session showed an
 * empty mode control. The last-seen lists are good enough to pick from;
 * the live ones replace them the moment the agent reports.
 */
export type AgentCaps = {
  config?: AcpConfigOption[];
  modes?: { current: string; available: AcpSessionMode[] };
};

const CAPS_KEY = 'mira.acp.caps.v2';
// v2: v1 attributed late frames from a stopped agent to whichever driver
// was current (`capsDriverRef`), so one agent's models could land in
// another's cache (OpenCode showing Claude models). Frames are now
// driver-stamped server-side; the bump drops any polluted v1 entries.

export function loadAgentCaps(driver: string): AgentCaps {
  try {
    const raw = localStorage.getItem(CAPS_KEY);
    if (!raw) return {};
    return (JSON.parse(raw) as Record<string, AgentCaps>)[driver] ?? {};
  } catch {
    return {};
  }
}

export function saveAgentCaps(driver: string, patch: AgentCaps) {
  try {
    const raw = localStorage.getItem(CAPS_KEY);
    const all = raw ? (JSON.parse(raw) as Record<string, AgentCaps>) : {};
    all[driver] = { ...(all[driver] ?? {}), ...patch };
    localStorage.setItem(CAPS_KEY, JSON.stringify(all));
  } catch {
    /* quota / private mode — the live lists still work */
  }
}

/** The model choices in an agent's config options, if it has a model option. */
export function agentModelChoices(config: AcpConfigOption[] | undefined | null): { value: string; label: string }[] {
  const opt = config?.find((o) => o.category === 'model');
  if (!opt) return [];
  return opt.values.map((v) => ({ value: v.value, label: v.name || v.value }));
}
