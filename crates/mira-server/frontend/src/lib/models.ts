/**
 * Model presentation: who makes a model, what to call it, which icon to
 * show, and the per-model option memory.
 *
 * Shared by the engine picker, the composer chip and the sidebar, so a
 * model reads the same everywhere. Pure data and string functions —
 * nothing here renders.
 */
import type { ModelInfo } from '../api';

/** Canonical vendor buckets. Model ids are matched on the model family
 *  (`claude-…`, `gpt-…`, `gemini-…`) as well as on a routing prefix
 *  (`anthropic/…`), because native ids carry no prefix and agent model
 *  aliases (`opus`, `sonnet`) carry neither. */
const FAMILY_RULES: [RegExp, string][] = [
  [/(^|\/)(anthropic|claude)|^(opus|sonnet|haiku|fable)(\b|\[|-|$)/i, 'anthropic'],
  [/(^|\/)(openai|gpt|chatgpt|codex)|(^|\/)o[1-9](-|$)/i, 'openai'],
  [/(^|\/)(google|gemini|gemma)/i, 'google'],
  [/(^|\/)(x-?ai|grok)/i, 'xai'],
  [/deepseek/i, 'deepseek'],
  [/(^|\/)(mistral|codestral|devstral|magistral|ministral|pixtral)/i, 'mistral'],
  [/(^|\/)(meta|llama)/i, 'meta'],
  [/qwen|qwq/i, 'qwen'],
  [/kimi|moonshot/i, 'moonshot'],
  [/(^|\/)(z-?ai|zhipu|glm)/i, 'zai'],
  [/minimax/i, 'minimax'],
  [/(^|\/)(cohere|command)/i, 'cohere'],
  [/perplexity|sonar/i, 'perplexity'],
  [/(^|\/)(microsoft|phi-)/i, 'microsoft'],
  [/(^|\/)(amazon|nova-)/i, 'amazon'],
  [/(^|\/)nous|hermes/i, 'nous'],
];

/** The vendor bucket a model id belongs to, or `other`. */
export function modelVendor(id: string | null | undefined): string {
  if (!id) return 'other';
  for (const [re, vendor] of FAMILY_RULES) if (re.test(id)) return vendor;
  return 'other';
}

/** Legacy prefix vendor (`openai/gpt-4o` → `openai`, `gpt-4o` → `gpt`),
 *  kept for grouping catalogs that route through an aggregator. */
export function vendorOf(id: string): string {
  if (!id) return '';
  const slash = id.indexOf('/');
  if (slash > 0) return id.slice(0, slash).toLowerCase();
  return id.split('-')[0].toLowerCase();
}

const VENDOR_BG: Record<string, string> = {
  openai: 'bg-emerald-500',
  anthropic: 'bg-orange-500',
  google: 'bg-blue-500',
  meta: 'bg-blue-600',
  mistral: 'bg-orange-600',
  deepseek: 'bg-indigo-500',
  xai: 'bg-neutral-200',
  qwen: 'bg-violet-500',
  microsoft: 'bg-sky-500',
  amazon: 'bg-amber-500',
  cohere: 'bg-rose-400',
  perplexity: 'bg-cyan-500',
  nous: 'bg-purple-500',
  moonshot: 'bg-yellow-400',
  zai: 'bg-slate-300',
  minimax: 'bg-pink-500',
  other: 'bg-neutral-500',
};

/** Dot colour for a model id. */
export function modelDotClass(id: string | null | undefined): string {
  return VENDOR_BG[modelVendor(id)] ?? VENDOR_BG.other;
}

const PRETTY: Record<string, string> = {
  openai: 'OpenAI', anthropic: 'Anthropic', google: 'Google', xai: 'xAI',
  meta: 'Meta', 'meta-llama': 'Meta', mistralai: 'Mistral', mistral: 'Mistral',
  deepseek: 'DeepSeek', qwen: 'Qwen', alibaba: 'Alibaba', amazon: 'Amazon',
  cohere: 'Cohere', groq: 'Groq', perplexity: 'Perplexity', microsoft: 'Microsoft',
  moonshotai: 'Moonshot AI', moonshot: 'Moonshot AI', openrouter: 'OpenRouter',
  'open-router': 'OpenRouter', huggingface: 'Hugging Face', together: 'Together',
  togetherai: 'Together', fireworks: 'Fireworks', databricks: 'Databricks',
  nvidia: 'NVIDIA', ollama: 'Ollama', lmstudio: 'LM Studio', zai: 'Z.ai',
  minimax: 'MiniMax', nous: 'Nous', other: 'Other',
};

export function prettyVendor(key: string): string {
  return PRETTY[key.toLowerCase()] ?? key.charAt(0).toUpperCase() + key.slice(1);
}

/** A model id as a label: routing prefix dropped, first letter raised. */
export function prettyModel(id: string | null | undefined): string {
  if (!id) return '';
  const slash = id.indexOf('/');
  const raw = slash > 0 ? id.slice(slash + 1) : id;
  return raw.length > 0 ? raw[0].toUpperCase() + raw.slice(1) : raw;
}

export function formatCtx(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(n % 1_000_000 ? 1 : 0)}M`;
  if (n >= 1000) return `${Math.round(n / 1000)}k`;
  return String(n);
}

/** Group a flat catalog by vendor, largest group first. */
export function groupByVendor(models: ModelInfo[]): { name: string; models: ModelInfo[] }[] {
  const byKey = new Map<string, ModelInfo[]>();
  for (const m of models) {
    const key = modelVendor(m.id);
    const bucket = byKey.get(key);
    if (bucket) bucket.push(m);
    else byKey.set(key, [m]);
  }
  return [...byKey.entries()]
    .map(([key, list]) => ({ name: prettyVendor(key), models: list }))
    .sort((a, b) => b.models.length - a.models.length);
}

/** Curated shortlist surfaced above the full list. Matched on id substring
 *  so it works across routers. Order is display order. */
export const CODING_MATCHERS: ((id: string) => boolean)[] = [
  (id) => /claude.*opus/i.test(id),
  (id) => /claude.*sonnet/i.test(id),
  (id) => /gpt-5/i.test(id),
  (id) => /gemini-[0-9.]+-pro/i.test(id),
  (id) => /grok-(code|4)/i.test(id),
  (id) => /deepseek.*(v3|r1)/i.test(id),
  (id) => /qwen.*coder/i.test(id),
  (id) => /kimi-k2/i.test(id),
];

// ---- brand icons ----------------------------------------------------------

/** Claude's mark, bundled. `claude.ai` refuses hotlinked favicons and
 *  Anthropic's own favicon is dark-on-transparent — invisible on this UI's
 *  dark surfaces — so neither network source can be trusted to show. */
export const CLAUDE_MARK =
  'data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A//www.w3.org/2000/svg%22%20viewBox%3D%220%200%2064%2064%22%3E%3Crect%20x%3D%224%22%20y%3D%224%22%20width%3D%2256%22%20height%3D%2256%22%20rx%3D%2213%22%20fill%3D%22%23D97757%22/%3E%3Cg%20stroke%3D%22%23fff%22%20stroke-width%3D%225.5%22%20stroke-linecap%3D%22round%22%3E%3Cline%20x1%3D%2232%22%20y1%3D%2232%22%20x2%3D%2252.0%22%20y2%3D%2232.0%22/%3E%3Cline%20x1%3D%2232%22%20y1%3D%2232%22%20x2%3D%2245.6%22%20y2%3D%2242.2%22/%3E%3Cline%20x1%3D%2232%22%20y1%3D%2232%22%20x2%3D%2242.0%22%20y2%3D%2249.3%22/%3E%3Cline%20x1%3D%2232%22%20y1%3D%2232%22%20x2%3D%2229.9%22%20y2%3D%2248.9%22/%3E%3Cline%20x1%3D%2232%22%20y1%3D%2232%22%20x2%3D%2222.0%22%20y2%3D%2249.3%22/%3E%3Cline%20x1%3D%2232%22%20y1%3D%2232%22%20x2%3D%2216.4%22%20y2%3D%2238.6%22/%3E%3Cline%20x1%3D%2232%22%20y1%3D%2232%22%20x2%3D%2212.0%22%20y2%3D%2232.0%22/%3E%3Cline%20x1%3D%2232%22%20y1%3D%2232%22%20x2%3D%2218.4%22%20y2%3D%2221.8%22/%3E%3Cline%20x1%3D%2232%22%20y1%3D%2232%22%20x2%3D%2222.0%22%20y2%3D%2214.7%22/%3E%3Cline%20x1%3D%2232%22%20y1%3D%2232%22%20x2%3D%2234.1%22%20y2%3D%2215.1%22/%3E%3Cline%20x1%3D%2232%22%20y1%3D%2232%22%20x2%3D%2242.0%22%20y2%3D%2214.7%22/%3E%3Cline%20x1%3D%2232%22%20y1%3D%2232%22%20x2%3D%2247.6%22%20y2%3D%2225.4%22/%3E%3C/g%3E%3C/svg%3E';

/** The vendor's own icon where it survives a dark ground; otherwise the
 *  domain for Google's favicon service. */
const VENDOR_ICON: Record<string, { direct?: string; domain: string }> = {
  anthropic: { direct: CLAUDE_MARK, domain: 'claude.ai' },
  openai: { direct: 'https://chatgpt.com/favicon.ico', domain: 'chatgpt.com' },
  google: { domain: 'gemini.google.com' },
  xai: { direct: 'https://x.ai/favicon.ico', domain: 'x.ai' },
  deepseek: { domain: 'deepseek.com' },
  mistral: { domain: 'mistral.ai' },
  meta: { domain: 'llama.com' },
  qwen: { domain: 'qwen.ai' },
  moonshot: { domain: 'kimi.com' },
  zai: { domain: 'z.ai' },
  minimax: { domain: 'minimax.io' },
  cohere: { domain: 'cohere.com' },
  perplexity: { domain: 'perplexity.ai' },
  microsoft: { domain: 'microsoft.com' },
  amazon: { domain: 'aws.amazon.com' },
  nous: { domain: 'nousresearch.com' },
};

/** Provider-instance icons, for rows that name a router rather than a model. */
const PROVIDER_ICON: Record<string, { direct?: string; domain: string }> = {
  anthropic: VENDOR_ICON.anthropic,
  openai: VENDOR_ICON.openai,
  google: VENDOR_ICON.google,
  gemini: VENDOR_ICON.google,
  xai: VENDOR_ICON.xai,
  openrouter: { direct: 'https://openrouter.ai/favicon.ico', domain: 'openrouter.ai' },
  groq: { domain: 'groq.com' },
  together: { domain: 'together.ai' },
  fireworks: { domain: 'fireworks.ai' },
  deepseek: VENDOR_ICON.deepseek,
  mistral: VENDOR_ICON.mistral,
  ollama: { domain: 'ollama.com' },
  bedrock: VENDOR_ICON.amazon,
  vertex: VENDOR_ICON.google,
};

function urlsFor(entry: { direct?: string; domain: string } | undefined): string[] {
  if (!entry) return [];
  const out: string[] = [];
  if (entry.direct) out.push(entry.direct);
  out.push(`https://www.google.com/s2/favicons?domain=${entry.domain}&sz=64`);
  return out;
}

/** Icon candidates for a model, best first. Empty → monogram. */
export function modelIconUrls(modelId: string | null | undefined): string[] {
  return urlsFor(VENDOR_ICON[modelVendor(modelId)]);
}

/** Providers whose mark is a dark glyph on a transparent ground (xAI's X,
 *  Ollama's llama, OpenRouter, ChatGPT): invisible on the dark theme unless drawn on a light
 *  tile. */
export const DARK_MARKS: ReadonlySet<string> = new Set(['xai', 'grok', 'ollama', 'openrouter', 'openai', 'lmstudio', 'llamacpp']);

/** Whether a provider instance's mark needs a light tile behind it. */
export function needsLightTile(instance: string | null | undefined): boolean {
  return DARK_MARKS.has((instance ?? '').toLowerCase().replace(/[-_](work|personal|\d+)$/, ''));
}

/** Icon candidates for a provider instance (`openrouter`, `anthropic`, …). */
export function providerIconUrls(instance: string | null | undefined, modelId?: string | null): string[] {
  const key = (instance ?? '').toLowerCase().replace(/[-_](work|personal|\d+)$/, '');
  const direct = PROVIDER_ICON[key];
  if (direct) return urlsFor(direct);
  return modelIconUrls(modelId);
}

// ---- per-model option memory ---------------------------------------------

/**
 * Remembered option values (effort, service tier, …) per model id. The
 * server is the authority on *which* options exist, so a stale entry for a
 * descriptor a model doesn't advertise is simply never rendered or sent.
 */
const OPTIONS_KEY = 'mira.model-options';

/** Options are remembered per engine instance and model (`groq:llama-…`),
 *  so two instances offering the same model id keep their own settings.
 *  Entries saved before that were keyed by model alone; they're the
 *  fallback, so existing picks carry over. */
function optionsKey(model: string, instance?: string | null): string {
  return instance ? `${instance}:${model}` : model;
}

export function loadModelOptions(model: string, instance?: string | null): Record<string, string> {
  try {
    const raw = localStorage.getItem(OPTIONS_KEY);
    if (!raw) return {};
    const map = JSON.parse(raw) as Record<string, Record<string, string>>;
    return map[optionsKey(model, instance)] ?? map[model] ?? {};
  } catch {
    return {};
  }
}

export function saveModelOptions(model: string, next: Record<string, string>, instance?: string | null) {
  try {
    const raw = localStorage.getItem(OPTIONS_KEY);
    const map = raw ? (JSON.parse(raw) as Record<string, Record<string, string>>) : {};
    map[optionsKey(model, instance)] = next;
    localStorage.setItem(OPTIONS_KEY, JSON.stringify(map));
  } catch {
    /* private mode — options still apply for this session */
  }
}
