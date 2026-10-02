/** Provider presets shared by Settings and onboarding. */

/**
 * Provider presets: name → base URL + a sensible default model for a
 * first turn. Every entry works today via the backend's OpenAI-compat
 * adapter (or the native Anthropic adapter for `anthropic`).
 *
 * Ordering roughly follows expected popularity for coding: gateways
 * first, then major hosted models, then hot new API providers, then
 * local runtimes at the bottom. If you add a provider here, mirror it
 * in `crates/mira-config/src/lib.rs::default_base_url_for` so the CLI
 * gets the same defaults, and in `default_api_key_env_for` if the
 * provider has a conventional env-var name.
 *
 * `suggested_model` is intentionally blank when I couldn't confirm a
 * coding-relevant default at ship time — shipping a stale model id
 * gives users a confusing 404 on their first turn; a blank field
 * makes them pick one on purpose.
 */
export const PROVIDER_PRESETS = [
  // Gateways
  { name: 'openrouter', base_url: 'https://openrouter.ai/api/v1',                     suggested_model: 'google/gemini-2.5-flash' },
  // Major hosted
  { name: 'openai',     base_url: 'https://api.openai.com/v1',                        suggested_model: 'gpt-4o-mini' },
  { name: 'anthropic',  base_url: 'https://api.anthropic.com/v1',                     suggested_model: 'claude-sonnet-4-5' },
  // Region comes from AWS_REGION or ~/.aws/config; the key is optional.
  { name: 'bedrock',    base_url: 'https://bedrock-runtime.amazonaws.com',            suggested_model: 'us.anthropic.claude-sonnet-4-5-20250929-v1:0' },
  { name: 'google',     base_url: 'https://generativelanguage.googleapis.com/v1beta/openai', suggested_model: 'gemini-2.5-flash' },
  // Fast / cheap inference
  { name: 'deepseek',   base_url: 'https://api.deepseek.com/v1',                      suggested_model: 'deepseek-chat' },
  { name: 'groq',       base_url: 'https://api.groq.com/openai/v1',                   suggested_model: 'moonshotai/kimi-k2-instruct' },
  { name: 'cerebras',   base_url: 'https://api.cerebras.ai/v1',                       suggested_model: '' },
  { name: 'xai',        base_url: 'https://api.x.ai/v1',                              suggested_model: 'grok-code-fast-1' },
  // Model bazaars
  { name: 'together',   base_url: 'https://api.together.xyz/v1',                      suggested_model: '' },
  { name: 'fireworks',  base_url: 'https://api.fireworks.ai/inference/v1',            suggested_model: '' },
  { name: 'hyperbolic', base_url: 'https://api.hyperbolic.xyz/v1',                    suggested_model: '' },
  { name: 'novita',     base_url: 'https://api.novita.ai/v3/openai',                  suggested_model: '' },
  // Search-augmented + regionals
  { name: 'perplexity', base_url: 'https://api.perplexity.ai',                        suggested_model: '' },
  { name: 'mistral',    base_url: 'https://api.mistral.ai/v1',                        suggested_model: 'codestral-latest' },
  { name: 'moonshot',   base_url: 'https://api.moonshot.ai/v1',                       suggested_model: '' },
  // Local runtimes
  { name: 'ollama',     base_url: 'http://localhost:11434/v1',                        suggested_model: 'llama3.1' },
  { name: 'lmstudio',   base_url: 'http://localhost:1234/v1',                         suggested_model: '' },
  { name: 'llamacpp',   base_url: 'http://localhost:8080/v1',                         suggested_model: '' },
];

/**
 * Providers that expose an OAuth PKCE sign-in flow. Users of these
 * providers can skip pasting an API key entirely — the dropdown surfaces
 * a "Sign in" badge so the affordance is discoverable without picking
 * each provider first, and the ProviderSection renders the matching
 * sign-in button once selected. Keep in sync with the OAuth handlers
 * registered in `mira-server/src/oauth/` (`openrouter.rs`, `openai.rs`).
 */
export const OAUTH_PROVIDERS: ReadonlySet<string> = new Set(['openrouter', 'openai']);

/** Provider homepage per preset, for favicons. Local runtimes use their
 *  public sites (their localhost base URLs have no icon to fetch). */
export const PROVIDER_FAVICON_DOMAIN: Record<string, string> = {
  openrouter: 'openrouter.ai',
  openai: 'openai.com',
  anthropic: 'claude.ai',
  bedrock: 'aws.amazon.com',
  google: 'cloud.google.com',
  deepseek: 'deepseek.com',
  groq: 'groq.com',
  cerebras: 'cerebras.ai',
  xai: 'x.ai',
  together: 'together.ai',
  fireworks: 'fireworks.ai',
  hyperbolic: 'hyperbolic.xyz',
  novita: 'novita.ai',
  perplexity: 'perplexity.ai',
  mistral: 'mistral.ai',
  moonshot: 'moonshot.ai',
  ollama: 'ollama.com',
  lmstudio: 'lmstudio.ai',
};
