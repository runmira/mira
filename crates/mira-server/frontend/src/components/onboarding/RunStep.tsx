/**
 * "How Mira runs": the two ways, side by side and kept distinct, because
 * they differ in what matters — who does the work and who you pay.
 *
 * - External agents: a coding agent you already use (Claude Code, Codex…)
 *   drives the chat, signed in with your own account. Nothing to set up;
 *   usage counts against that agent's plan.
 * - Providers: Mira's own agent, on a model you choose, through your API
 *   key. You pay the provider per token; Mira's subagents, goals and code
 *   review run on it.
 *
 * Either is enough to start, both is fine (pick per chat), neither is
 * allowed (skip; the app asks again when a chat needs one).
 */
import { useEffect, useState } from 'react';
import { Check, Copy, KeyRound, Loader2, Terminal } from 'lucide-react';
import { getSettings, listEngines, putSettings, type EngineSnapshot } from '../../api';
import type { SettingsView } from '../../types';
import { PROVIDER_PRESETS } from '../../lib/providers';
import { AgentIcon, ProviderIcon } from '../AgentIcon';
import { PrivateText } from '../PrivateText';
import { cn } from '@/lib/utils';
import { ErrorText, GhostButton, PrimaryButton, StepFooter, StepHeader, field, tile } from './ui';

/** The providers offered here; the rest are a click away in Settings. */
const OFFERED = ['openrouter', 'anthropic', 'openai', 'google', 'groq', 'ollama'] as const;
const PROVIDER_LABEL: Record<string, string> = {
  openrouter: 'OpenRouter',
  anthropic: 'Anthropic',
  openai: 'OpenAI',
  google: 'Google Gemini',
  groq: 'Groq',
  ollama: 'Ollama (local)',
};
const PROVIDER_NOTE: Record<string, string> = {
  openrouter: 'Every major model, one key',
  anthropic: 'Claude models',
  openai: 'GPT models',
  google: 'Gemini models',
  groq: 'Very fast open models',
  ollama: 'Models on this computer, no key',
};
const KEY_URL: Record<string, string> = {
  openrouter: 'https://openrouter.ai/keys',
  anthropic: 'https://console.anthropic.com/settings/keys',
  openai: 'https://platform.openai.com/api-keys',
  google: 'https://aistudio.google.com/apikey',
  groq: 'https://console.groq.com/keys',
};

/** Agents shown even when absent, because they're the common ones. */
const FEATURED_AGENTS = ['claude-code', 'codex'];

function agentStatus(e: EngineSnapshot): 'ready' | 'missing' | 'problem' | 'checking' {
  if (e.state.state === 'ready') return 'ready';
  if (e.state.state === 'not_found') return 'missing';
  if (e.state.state === 'failed' && /checking/i.test(e.state.reason)) return 'checking';
  return 'problem';
}

function CopyLine({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      onClick={() => void navigator.clipboard.writeText(text).then(() => { setCopied(true); setTimeout(() => setCopied(false), 1200); })}
      className="group mt-2 flex w-full items-center gap-2 rounded-lg border border-white/[0.06] bg-black/40 px-2.5 py-1.5 text-left font-mono text-[11px] text-foreground/75 transition-colors hover:text-foreground"
      title="Copy"
    >
      <span className="min-w-0 flex-1 truncate">{text}</span>
      {copied ? <Check className="size-3 text-emerald-400" /> : <Copy className="size-3 opacity-50 group-hover:opacity-100" />}
    </button>
  );
}

export function RunStep({ onNext, onBack }: { onNext: () => void; onBack?: () => void }) {
  const [engines, setEngines] = useState<EngineSnapshot[] | null>(null);
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [picked, setPicked] = useState<string | null>(null);
  const [key, setKey] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Agents are probed in the background on the server: poll until the
  // answer is in, briefly.
  useEffect(() => {
    let live = true;
    let tries = 0;
    const tick = () =>
      listEngines(tries === 0).then((v) => {
        if (!live) return;
        setEngines(v.engines);
        tries += 1;
        if (!v.fresh && tries < 12) setTimeout(tick, 1200);
      });
    void tick();
    void getSettings().then((s) => live && setSettings(s)).catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  const agents = (engines ?? [])
    .filter((e) => e.flavor === 'external')
    .filter((e) => FEATURED_AGENTS.includes(e.driver) || agentStatus(e) === 'ready')
    .sort((a, b) => Number(agentStatus(b) === 'ready') - Number(agentStatus(a) === 'ready'));
  const connected = new Set(
    (settings?.providers ?? []).filter((p) => p.has_api_key || p.name === 'ollama').map((p) => p.name),
  );
  const anyAgent = agents.some((a) => agentStatus(a) === 'ready');
  const anyProvider = (settings?.configured ?? false) || connected.size > 0;

  async function saveProvider() {
    if (!picked) return;
    const preset = PROVIDER_PRESETS.find((p) => p.name === picked);
    if (!preset) return;
    const needsKey = picked !== 'ollama';
    if (needsKey && !key.trim()) {
      setError('Paste your API key first.');
      return;
    }
    setError(null);
    setSaving(true);
    try {
      const view = await putSettings({
        default_provider: picked,
        ...(preset.suggested_model ? { default_model: preset.suggested_model } : {}),
        providers: [{ name: picked, base_url: preset.base_url, ...(needsKey ? { api_key: key.trim() } : {}) }],
      });
      setSettings(view);
      setKey('');
      setPicked(null);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="flex flex-col gap-7">
      <StepHeader eyebrow="Engines" title="How should Mira run?">
        Two different things: an <span className="text-foreground">external agent</span> you already use, or a{' '}
        <span className="text-foreground">provider</span> that powers Mira's own agent. Set up either — or both, and pick
        per chat.
      </StepHeader>

      <div className="grid gap-4 md:grid-cols-2">
        {/* External agents */}
        <div className={cn(tile, 'relative flex flex-col overflow-hidden p-5')}>
          <span className="pointer-events-none absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-mira-blue/60 to-transparent" />
          <div className="flex items-center gap-2.5">
            <span className="grid size-9 place-items-center rounded-xl border border-mira-blue/25 bg-mira-blue/12 text-mira-blue">
              <Terminal className="size-[18px]" />
            </span>
            <div className="min-w-0 flex-1">
              <div className="text-[14px] font-semibold text-foreground">External agents</div>
              <div className="text-[11.5px] text-muted-foreground">Your own plan · no API key</div>
            </div>
          </div>
          <p className="mt-3 text-[12.5px] leading-relaxed text-muted-foreground">
            Claude Code, Codex and others drive the chat with their own tools, signed in to your account. Usage counts against
            that agent's subscription.
          </p>
          <div className="mt-3 flex flex-col gap-2">
            {engines === null ? (
              <div className="flex items-center gap-2 py-2 text-[12.5px] text-muted-foreground">
                <Loader2 className="size-3.5 animate-spin" /> Looking for agents on this computer…
              </div>
            ) : (
              agents.map((a) => {
                const st = agentStatus(a);
                return (
                  <div key={a.instance} className="rounded-xl border border-white/[0.07] bg-black/30 px-3 py-2.5">
                    <div className="flex items-center gap-2.5">
                      <AgentIcon kind={a.driver} name={a.display_name} size="sm" />
                      <span className="min-w-0 flex-1">
                        <span className="block text-[13px] font-medium text-foreground">{a.display_name}</span>
                        {st === 'ready' && a.auth && (
                          <span className="block truncate text-[11px] text-muted-foreground">
                            <PrivateText text={a.auth} />
                          </span>
                        )}
                      </span>
                      <span
                        className={cn(
                          'shrink-0 rounded-full px-2 py-0.5 text-[11px] font-medium',
                          st === 'ready' && 'bg-emerald-500/12 text-emerald-400',
                          st === 'missing' && 'bg-white/[0.06] text-muted-foreground',
                          st === 'problem' && 'bg-amber-500/12 text-amber-300',
                          st === 'checking' && 'bg-white/[0.06] text-muted-foreground',
                        )}
                      >
                        {st === 'ready' ? 'Ready' : st === 'missing' ? 'Not installed' : st === 'checking' ? 'Checking…' : 'Needs attention'}
                      </span>
                    </div>
                    {st === 'missing' && a.install_hint && <CopyLine text={a.install_hint} />}
                    {st === 'problem' && 'reason' in a.state && (
                      <p className="mt-1.5 text-[11px] leading-snug text-amber-200/80">{a.state.reason}</p>
                    )}
                  </div>
                );
              })
            )}
          </div>
        </div>

        {/* Providers */}
        <div className={cn(tile, 'relative flex flex-col overflow-hidden p-5')}>
          <span className="pointer-events-none absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-amber-300/60 to-transparent" />
          <div className="flex items-center gap-2.5">
            <span className="grid size-9 place-items-center rounded-xl border border-amber-300/25 bg-amber-400/12 text-amber-300">
              <KeyRound className="size-[18px]" />
            </span>
            <div className="min-w-0 flex-1">
              <div className="text-[14px] font-semibold text-foreground">Providers</div>
              <div className="text-[11.5px] text-muted-foreground">Your API key · pay per use</div>
            </div>
          </div>
          <p className="mt-3 text-[12.5px] leading-relaxed text-muted-foreground">
            Mira's own agent on any model you choose. You pay the provider for what you use; Mira's subagents, goals and code
            review run on it.
          </p>
          <div className="mt-3 grid grid-cols-2 gap-2">
            {OFFERED.map((name) => {
              const on = connected.has(name);
              return (
                <button
                  key={name}
                  type="button"
                  onClick={() => {
                    setPicked(picked === name ? null : name);
                    setError(null);
                  }}
                  className={cn(
                    'flex items-center gap-2 rounded-xl border px-2.5 py-2 text-left transition-colors',
                    picked === name
                      ? 'border-amber-400/60 bg-amber-400/[0.08] shadow-[0_0_0_3px_rgba(251,191,36,0.1)]'
                      : 'border-white/[0.07] bg-black/30 hover:border-white/[0.16] hover:bg-white/[0.04]',
                  )}
                >
                  <ProviderIcon instance={name} name={PROVIDER_LABEL[name]} size="xs" />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[12.5px] font-medium text-foreground">{PROVIDER_LABEL[name]}</span>
                    <span className="block truncate text-[10.5px] text-muted-foreground">{on ? 'Connected' : PROVIDER_NOTE[name]}</span>
                  </span>
                  {on && <Check className="size-3.5 shrink-0 text-emerald-400" />}
                </button>
              );
            })}
          </div>
          {picked && (
            <div className="onb-in mt-3 rounded-xl border border-white/[0.07] bg-black/30 p-3">
              {picked === 'ollama' ? (
                <p className="text-[12px] leading-relaxed text-muted-foreground">
                  Uses Ollama running on this computer at <span className="font-mono text-foreground/80">localhost:11434</span>. No key needed.
                </p>
              ) : (
                <>
                  <label className="text-[12px] text-muted-foreground" htmlFor="onb-key">
                    {PROVIDER_LABEL[picked]} API key
                  </label>
                  <input
                    id="onb-key"
                    type="password"
                    autoFocus
                    value={key}
                    onChange={(e) => setKey(e.target.value)}
                    onKeyDown={(e) => e.key === 'Enter' && void saveProvider()}
                    placeholder="Paste your key"
                    className={cn(field, 'mt-1.5 h-10 font-mono text-[12.5px] focus:border-amber-400/60 focus:ring-amber-400/15')}
                  />
                  {KEY_URL[picked] && (
                    <a href={KEY_URL[picked]} target="_blank" rel="noreferrer" className="mt-1.5 inline-block text-[11.5px] text-muted-foreground underline underline-offset-2 hover:text-foreground">
                      Get a key
                    </a>
                  )}
                  <p className="mt-1 text-[11px] text-muted-foreground/70">Stored on this computer only.</p>
                </>
              )}
              {error && (
                <div className="mt-2">
                  <ErrorText>{error}</ErrorText>
                </div>
              )}
              <button
                type="button"
                disabled={saving}
                onClick={() => void saveProvider()}
                className="mt-2.5 h-9 w-full rounded-lg bg-amber-300 text-[12.5px] font-semibold text-[#1a1b26] transition-[filter,opacity] hover:brightness-105 disabled:opacity-50"
              >
                {saving ? 'Saving…' : `Use ${PROVIDER_LABEL[picked]}`}
              </button>
            </div>
          )}
          <p className="mt-3 text-[11px] text-muted-foreground/70">Bedrock, DeepSeek, Mistral, xAI and more are in Settings → Provider.</p>
        </div>
      </div>

      <StepFooter
        onBack={onBack}
        status={
          anyAgent && anyProvider
            ? 'Both set up — switch per chat from the model picker.'
            : anyAgent
              ? 'Ready with an external agent.'
              : anyProvider
                ? 'Ready with a provider.'
                : 'Set up one to start, or skip for now.'
        }
      >
        {anyAgent || anyProvider ? (
          <PrimaryButton onClick={onNext}>Continue</PrimaryButton>
        ) : (
          <GhostButton onClick={onNext}>Skip for now</GhostButton>
        )}
      </StepFooter>
    </section>
  );
}
