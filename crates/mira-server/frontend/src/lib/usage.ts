// Client-side pricing + token formatting. Mirrors crates/mira-ai/src/pricing.rs.
//
// Keep the two in sync — the backend is authoritative for cost accounting;
// this exists so the UI can render `$0.024` without a round-trip. Unknown
// models fall through to "tokens-only" rendering.

import type { TokenUsage, UsageTotals } from '../types';

type Price = {
  input: number;         // USD per 1M tokens
  output: number;
  cached_input: number;  // usually cheaper than fresh input
};

// Prefix-match — longer prefixes first, so `gpt-4o-mini` beats `gpt-4o`.
const MODEL_PRICES: [string, Price][] = [
  ['gpt-4o-mini',   { input: 0.15,  output: 0.60,  cached_input: 0.075 }],
  ['gpt-4o',        { input: 2.50,  output: 10.00, cached_input: 1.25  }],
  ['gpt-4.1-mini',  { input: 0.40,  output: 1.60,  cached_input: 0.10  }],
  ['gpt-4.1',       { input: 2.00,  output: 8.00,  cached_input: 0.50  }],
  ['o1-mini',       { input: 3.00,  output: 12.00, cached_input: 1.50  }],
  ['o1',            { input: 15.00, output: 60.00, cached_input: 7.50  }],
  ['o3-mini',       { input: 1.10,  output: 4.40,  cached_input: 0.55  }],
  ['o3',            { input: 2.00,  output: 8.00,  cached_input: 0.50  }],

  ['claude-haiku-4',  { input: 1.00,  output: 5.00,  cached_input: 0.10 }],
  ['claude-sonnet-4', { input: 3.00,  output: 15.00, cached_input: 0.30 }],
  ['claude-opus-4',   { input: 15.00, output: 75.00, cached_input: 1.50 }],

  ['llama-3.3-70b',  { input: 0.59, output: 0.79, cached_input: 0.59 }],
  ['llama-3.1-70b',  { input: 0.59, output: 0.79, cached_input: 0.59 }],
  ['llama-3.1-8b',   { input: 0.05, output: 0.08, cached_input: 0.05 }],
  ['qwen-2.5-72b',   { input: 0.90, output: 0.90, cached_input: 0.90 }],
  ['deepseek-v3',    { input: 0.27, output: 1.10, cached_input: 0.07 }],
];

export function priceFor(model: string): Price | null {
  const m = model.toLowerCase();
  const hit = MODEL_PRICES.find(([prefix]) => m.startsWith(prefix));
  return hit ? hit[1] : null;
}

export function costUsd(model: string, u: TokenUsage | UsageTotals): number | null {
  const p = priceFor(model);
  if (!p) return null;
  const uncachedInput = Math.max(0, u.prompt_tokens - u.cached_input_tokens);
  return (
    (uncachedInput * p.input) / 1_000_000 +
    (u.cached_input_tokens * p.cached_input) / 1_000_000 +
    (u.completion_tokens * p.output) / 1_000_000
  );
}

/** Compact form: `12.3k`, `1.24M`, or raw for < 1000. */
export function shortNum(n: number): string {
  if (n < 1_000) return String(n);
  if (n < 1_000_000) return `${(n / 1_000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(2)}M`;
}

export function formatDollars(d: number): string {
  if (d < 0.01) return `$${d.toFixed(4)}`;
  if (d < 1)    return `$${d.toFixed(3)}`;
  return `$${d.toFixed(2)}`;
}

/**
 * `↑12.3k ↓4.1k · $0.024` — or just the token line if pricing is unknown.
 * Returns null when there's nothing to show (empty totals).
 */
export function formatUsage(model: string, u: UsageTotals | null | undefined): string | null {
  if (!u) return null;
  if (u.prompt_tokens === 0 && u.completion_tokens === 0) return null;
  let s = `↑${shortNum(u.prompt_tokens)} ↓${shortNum(u.completion_tokens)}`;
  const cost = costUsd(model, u);
  if (cost != null) s += ` · ${formatDollars(cost)}`;
  return s;
}
