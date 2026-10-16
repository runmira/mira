/**
 * Usage rows (`/api/usage`, one per session/day/model) rolled up for the
 * Usage page: totals, per-provider totals, a per-day per-provider series
 * for the chart, a per-model table, and cost and tokens split by type.
 *
 * Cost is the API estimate: an agent's own figure when it reported one,
 * otherwise the price table. Turns covered by a subscription are included
 * (and counted separately in `subscriptionUsd`) — the page is about what
 * the work is worth, not the bill.
 */
import { costUsd, priceFor } from '../../lib/usage';
import { providerKey } from '../../lib/usageProviders';

export type UsageRow = {
  session_id: string;
  title: string | null;
  cwd: string;
  model: string;
  day: string;
  prompt_tokens: number;
  completion_tokens: number;
  cached_input_tokens: number;
  /** The external agent that ran these turns (`claude`, `codex`). */
  agent?: string | null;
  /** The agent's own cost estimate, used instead of the price table. */
  cost_usd?: number | null;
  billing?: 'api' | 'subscription' | 'unknown' | null;
};

export type Totals = {
  totalTokens: number;
  cachedInput: number;
  uncachedInput: number;
  output: number;
  costUsd: number;
  /** Cost of tokens with no known price: none, but they exist. */
  unpricedTokens: number;
  subscriptionUsd: number;
  sessions: number;
};

export type ModelRow = Totals & { key: string; provider: string; model: string };
export type DayTotals = { day: string; byProvider: Map<string, Totals>; total: Totals };

function empty(): Totals & { sessionIds: Set<string> } {
  return {
    totalTokens: 0, cachedInput: 0, uncachedInput: 0, output: 0, costUsd: 0,
    unpricedTokens: 0, subscriptionUsd: 0, sessions: 0, sessionIds: new Set(),
  };
}

export function rowCost(r: UsageRow): number | null {
  if (typeof r.cost_usd === 'number') return r.cost_usd;
  return costUsd(r.model, {
    prompt_tokens: r.prompt_tokens,
    completion_tokens: r.completion_tokens,
    cached_input_tokens: r.cached_input_tokens,
    rounds: 0,
  });
}

function add(t: Totals & { sessionIds: Set<string> }, r: UsageRow) {
  const cached = Math.min(r.cached_input_tokens, r.prompt_tokens);
  t.cachedInput += cached;
  t.uncachedInput += r.prompt_tokens - cached;
  t.output += r.completion_tokens;
  t.totalTokens += r.prompt_tokens + r.completion_tokens;
  const c = rowCost(r);
  if (c == null) t.unpricedTokens += r.prompt_tokens + r.completion_tokens;
  else {
    t.costUsd += c;
    if (r.billing === 'subscription') t.subscriptionUsd += c;
  }
  t.sessionIds.add(r.session_id);
  t.sessions = t.sessionIds.size;
}

function finish<T extends Totals & { sessionIds: Set<string> }>(t: T): Omit<T, 'sessionIds'> {
  const { sessionIds: _ids, ...rest } = t;
  return rest;
}

export type UsageModel = {
  total: Totals;
  byProvider: Map<string, Totals>;
  daily: DayTotals[];
  models: ModelRow[];
  /** Cost split by token type (priced-by-table rows only: an agent's own
   *  figure has no split), and what caching saved against uncached input. */
  costByType: { uncachedInput: number; cachedInput: number; output: number };
  cacheSavingsUsd: number;
};

export function buildUsage(rows: UsageRow[], days: string[]): UsageModel {
  const total = empty();
  const byProvider = new Map<string, ReturnType<typeof empty>>();
  const byDay = new Map<string, { byProvider: Map<string, ReturnType<typeof empty>>; total: ReturnType<typeof empty> }>();
  const byModel = new Map<string, ReturnType<typeof empty> & { key: string; provider: string; model: string }>();
  const costByType = { uncachedInput: 0, cachedInput: 0, output: 0 };
  let cacheSavingsUsd = 0;

  for (const r of rows) {
    const provider = providerKey(r.agent);
    add(total, r);
    if (!byProvider.has(provider)) byProvider.set(provider, empty());
    add(byProvider.get(provider)!, r);
    if (!byDay.has(r.day)) byDay.set(r.day, { byProvider: new Map(), total: empty() });
    const day = byDay.get(r.day)!;
    add(day.total, r);
    if (!day.byProvider.has(provider)) day.byProvider.set(provider, empty());
    add(day.byProvider.get(provider)!, r);
    const key = `${provider}:${r.model}`;
    if (!byModel.has(key)) byModel.set(key, { ...empty(), key, provider, model: r.model });
    add(byModel.get(key)!, r);

    const p = typeof r.cost_usd === 'number' ? null : priceFor(r.model);
    if (p) {
      const cached = Math.min(r.cached_input_tokens, r.prompt_tokens);
      costByType.uncachedInput += ((r.prompt_tokens - cached) * p.input) / 1_000_000;
      costByType.cachedInput += (cached * p.cached_input) / 1_000_000;
      costByType.output += (r.completion_tokens * p.output) / 1_000_000;
      cacheSavingsUsd += (cached * Math.max(0, p.input - p.cached_input)) / 1_000_000;
    }
  }

  return {
    total: finish(total),
    byProvider: new Map([...byProvider].map(([k, v]) => [k, finish(v)])),
    daily: days.map((day) => {
      const d = byDay.get(day);
      return {
        day,
        byProvider: new Map([...(d?.byProvider ?? [])].map(([k, v]) => [k, finish(v)])),
        total: finish(d?.total ?? empty()),
      };
    }),
    models: [...byModel.values()].map(finish).sort((a, b) => b.costUsd - a.costUsd || b.totalTokens - a.totalTokens),
    costByType,
    cacheSavingsUsd,
  };
}

/** `count` days ending with `until` (UTC `YYYY-MM-DD`), oldest first. */
export function dayRange(since: string, count: number): string[] {
  const out: string[] = [];
  const d = new Date(`${since}T00:00:00Z`);
  for (let i = 0; i < count; i++) {
    out.push(d.toISOString().slice(0, 10));
    d.setUTCDate(d.getUTCDate() + 1);
  }
  return out;
}

export function formatTokens(n: number): string {
  if (n < 1_000) return String(Math.round(n));
  if (n < 1_000_000) return `${(n / 1_000).toFixed(n < 10_000 ? 1 : 0)}K`;
  if (n < 1_000_000_000) return `${(n / 1_000_000).toFixed(n < 10_000_000 ? 2 : 1)}M`;
  return `${(n / 1_000_000_000).toFixed(2)}B`;
}

export function formatUsd(n: number): string {
  if (n === 0) return '$0';
  if (n < 0.01) return `$${n.toFixed(4)}`;
  if (n < 1000) return `$${n.toFixed(2)}`;
  return `$${Math.round(n).toLocaleString('en-US')}`;
}

export function formatPercent(share: number): string {
  if (share <= 0) return '0%';
  if (share < 0.01) return '<1%';
  return `${Math.round(share * 100)}%`;
}

/** `Oct 16` from `2026-10-16`. */
export function formatDayShort(day: string): string {
  const d = new Date(`${day}T00:00:00Z`);
  return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric', timeZone: 'UTC' });
}
