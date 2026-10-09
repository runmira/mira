import { SectionInput } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { cn } from '@/lib/utils';
import { Check, Plug } from 'lucide-react';
import React, { useState } from 'react';
import chatgptIcon from '../../assets/chatgpt-icon.svg';
import openrouterIcon from '../../assets/openrouter-icon.png';
import { needsLightTile } from '../../lib/models';
import { OAUTH_PROVIDERS, PROVIDER_FAVICON_DOMAIN, PROVIDER_PRESETS } from '../../lib/providers';
import type { SettingsView } from '../../types';
import { ApiKeyControl, KeyStatus } from './KeysSection';
import { TRow, TSection } from './SettingsFields';
import { Draft } from './types';
/**
 * Two-pane settings: sidebar of sections on the left, the active section's
 * fields on the right. State is pooled at this level so every section
 * edits the same `draft` and a single "Save" commits everything at once —
 * users can jump between sections without losing work.
 */

/** Brand mark for a provider preset: bundled art for the OAuth providers,
 *  live site favicon otherwise (hidden if it fails to load, e.g. offline). */
export function ProviderIcon({ name }: { name: string }) {
  const [failed, setFailed] = useState(false);
  const bundled = name === 'openrouter' ? openrouterIcon : name === 'openai' ? chatgptIcon : null;
  if (bundled) {
    return (
      <img
        src={bundled}
        alt=""
        aria-hidden="true"
        draggable={false}
        className="size-4 shrink-0 rounded-[4px] object-contain"
      />
    );
  }
  const domain = PROVIDER_FAVICON_DOMAIN[name];
  if (!domain || failed) return null;
  const img = (
    <img
      src={`https://www.google.com/s2/favicons?domain=${domain}&sz=64`}
      alt=""
      aria-hidden="true"
      draggable={false}
      onError={() => setFailed(true)}
      className={
        needsLightTile(name)
          ? 'size-3 object-contain'
          : 'size-4 shrink-0 rounded-[4px] object-contain'
      }
    />
  );
  // A dark mark (xAI, Ollama) disappears on the dark theme without a light
  // ground under it.
  return needsLightTile(name) ? (
    <span className="grid size-4 shrink-0 place-items-center rounded-[4px] bg-white">{img}</span>
  ) : (
    img
  );
}

/* ---------- section: provider ---------- */

export function ProviderSection({
  view,
  draft,
  setDraft,
  refetch,
}: {
  view: SettingsView;
  draft: Draft;
  setDraft: (u: (d: Draft) => Draft) => void;
  refetch: () => void;
}) {
  const preset = PROVIDER_PRESETS.find((x) => x.name === draft.providerName);
  const stored = view.providers.find((x) => x.name === draft.providerName);

  function onProviderChange(next: string) {
    const p = PROVIDER_PRESETS.find((x) => x.name === next);
    const s = view.providers.find((x) => x.name === next);
    setDraft((d) => ({
      ...d,
      providerName: next,
      baseUrl: s?.base_url ?? p?.base_url ?? '',
      apiKey: '',
      showReplaceKey: !(s?.has_api_key || s?.api_key_env),
      model: !s?.has_api_key && p?.suggested_model ? p.suggested_model : d.model,
    }));
  }

  const keyStatus: KeyStatus = (() => {
    if (!stored) return { kind: 'none' };
    if (stored.has_api_key && stored.api_key_masked) {
      return { kind: 'literal', masked: stored.api_key_masked };
    }
    if (stored.api_key_env) return { kind: 'env', name: stored.api_key_env };
    return { kind: 'none' };
  })();

  return (
    <TSection
      icon={<Plug className="size-3.5" />}
      title="Model provider"
      description="Where Mira sends chat requests. All providers speak the OpenAI-compatible /chat/completions wire."
    >
      <TRow
        title="Provider"
        description="Preset endpoints — you can override the base URL below."
        control={
          <Select
            value={draft.providerName}
            onChange={onProviderChange}
            options={PROVIDER_PRESETS.map((p) => ({
              value: p.name,
              label: p.name,
              hint: p.base_url,
              icon: <ProviderIcon name={p.name} />,
              // Surface OAuth support up-front so users don't have to
              // pick each provider one-by-one to discover that
              // OpenRouter / OpenAI let them skip pasting an API key.
              badge: OAUTH_PROVIDERS.has(p.name) ? 'Sign in' : undefined,
            }))}
            className="h-8 w-full text-[13px] sm:w-64"
          />
        }
      />

      <TRow
        title="Base URL"
        control={
          <SectionInput
            value={draft.baseUrl}
            onChange={(e) => setDraft((d) => ({ ...d, baseUrl: e.target.value }))}
            placeholder={preset?.base_url}
            spellCheck={false}
            className="h-8 text-[13px] sm:w-64"
          />
        }
      />

      {/* OAuth sign-in gets the full row width: the panel carries its
       *  own status badge, CTA and explanatory copy, which would squeeze
       *  the title column if crammed into the control slot. Non-OAuth
       *  providers keep the compact paste-a-key control on the right. */}
      {OAUTH_PROVIDERS.has(draft.providerName) ? (
        <TRow title="Sign in">
          <div className="mt-2.5">
            <OauthProviderPanel
              providerName={draft.providerName}
              keyStatus={keyStatus}
              onSignedIn={refetch}
            />
          </div>
        </TRow>
      ) : (
        <TRow
          title="API key"
          description="Stored in your mira.yaml. Never sent to the model."
          control={
            <ApiKeyControl
              status={keyStatus}
              showInput={draft.showReplaceKey}
              value={draft.apiKey}
              onChange={(v) => setDraft((d) => ({ ...d, apiKey: v }))}
              onReplace={() => setDraft((d) => ({ ...d, showReplaceKey: true, apiKey: '' }))}
              onCancelReplace={() => setDraft((d) => ({ ...d, showReplaceKey: false, apiKey: '' }))}
            />
          }
        />
      )}
      {draft.providerName === 'bedrock' && (
        <TRow
          title="AWS credentials"
          description="A Bedrock API key is optional. Without one, Mira signs requests with your AWS credentials (AWS_ACCESS_KEY_ID or ~/.aws/credentials). Put the region in the base URL, e.g. https://bedrock-runtime.us-west-2.amazonaws.com, or set AWS_REGION."
        />
      )}

      <TRow
        title="Model"
        description="The specific model id sent with each request."
        control={
          <SectionInput
            value={draft.model}
            onChange={(e) => setDraft((d) => ({ ...d, model: e.target.value }))}
            placeholder={preset?.suggested_model}
            spellCheck={false}
            className="h-8 text-[13px] sm:w-64"
          />
        }
      />

      <TRow
        title="Small model"
        description="Optional. A cheaper, faster model on the same provider for background work: session titles, context summaries, memory, and helper agents set to model: small (or haiku). Leave empty to use the main model."
        control={
          <SectionInput
            value={draft.smallModel}
            onChange={(e) => setDraft((d) => ({ ...d, smallModel: e.target.value }))}
            placeholder="e.g. a Haiku, mini or flash model"
            spellCheck={false}
            className="h-8 text-[13px] sm:w-64"
          />
        }
      />
    </TSection>
  );
}

/** Per-provider OAuth affordances — label, icon, description. Kept
 *  as a small config table so adding a third OAuth provider later
 *  needs only one entry, not another wrapper component. */
export type OauthSpec = {
  label: string;
  icon: string;
  description: React.ReactNode;
};

export const OAUTH_SPECS: Record<string, OauthSpec> = {
  openrouter: {
    label: 'Sign in with OpenRouter',
    icon: openrouterIcon,
    description: (
      <>
        Sign in with your OpenRouter account and we'll receive a key automatically — stored in your
        local <code>mira.yaml</code>.
      </>
    ),
  },
  openai: {
    label: 'Sign in with ChatGPT',
    icon: chatgptIcon,
    description: (
      <>
        Use your ChatGPT Plus / Pro / Team subscription instead of an API key. Mira refreshes the
        underlying token in the background so long sessions don't get logged out.
      </>
    ),
  },
};

/**
 * OAuth sign-in panel for providers that expose a PKCE flow. When
 * `keyStatus` says a key is already stored (signed in), we show a
 * compact "Signed in" chip with a "Sign in again" link. Otherwise we
 * show the primary white-pill CTA.
 *
 * The polling approach (repeated `GET /api/settings` until
 * `has_api_key` flips) avoids adding a WebSocket/EventSource just for
 * this; a user who bails part-way just leaves the panel in `awaiting`
 * until the 3-min ceiling — no long-term harm.
 */
export function OauthProviderPanel({
  providerName,
  keyStatus,
  onSignedIn,
}: {
  providerName: string;
  keyStatus: { kind: 'literal' | 'env' | 'none'; masked?: string; name?: string };
  onSignedIn: () => void;
}) {
  const spec = OAUTH_SPECS[providerName];
  const [status, setStatus] = useState<'idle' | 'awaiting' | 'error'>('idle');
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const alreadySignedIn = keyStatus.kind === 'literal';

  async function begin() {
    setErrorMsg(null);
    setStatus('awaiting');
    try {
      const resp = await fetch(`/api/auth/${providerName}/start`, { method: 'POST' });
      if (!resp.ok) throw new Error(`start ${resp.status}`);
      const { authorize_url } = (await resp.json()) as { authorize_url: string };
      window.open(authorize_url, '_blank', 'noopener,noreferrer');
      const deadline = Date.now() + 3 * 60 * 1000;
      while (Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, 1500));
        try {
          const s = (await fetch('/api/settings').then((r) => r.json())) as SettingsView;
          const p = s.providers.find((x) => x.name === providerName);
          if (p?.has_api_key) {
            setStatus('idle');
            onSignedIn();
            return;
          }
        } catch {
          /* transient — keep polling */
        }
      }
      setStatus('error');
      setErrorMsg('Sign-in timed out. Try again in a new tab.');
    } catch (e) {
      setStatus('error');
      setErrorMsg(e instanceof Error ? e.message : String(e));
    }
  }

  if (!spec) return null;

  return (
    <div className="flex flex-col gap-2">
      {alreadySignedIn ? (
        <div className="flex items-center gap-3 rounded-md border border-emerald-500/40 bg-emerald-500/[0.06] px-3 py-2 text-[12.5px]">
          <Check strokeWidth={2.5} className="size-4 shrink-0 text-emerald-400" />
          <div className="flex-1 min-w-0">
            <div className="text-foreground">Signed in</div>
            <div className="mt-0.5 truncate font-mono text-[11px] text-muted-foreground">
              {keyStatus.masked}
            </div>
          </div>
          <SignInPill spec={spec} status={status} onClick={begin} label="Sign in again" />
        </div>
      ) : (
        <SignInPill spec={spec} status={status} onClick={begin} label={spec.label} />
      )}
      {status === 'error' && errorMsg && (
        <div className="text-[11.5px] text-destructive">{errorMsg}</div>
      )}
      <div className="text-[11.5px] text-muted-foreground">{spec.description}</div>
    </div>
  );
}

/**
 * The primary CTA: white-bg capsule with the provider mark on the
 * left. Feels distinct from Mira's own outline/secondary buttons so
 * "sign in with X" reads as an *action taken with X's brand*, not a
 * generic form control. Bg stays white when disabled (mid-flow) but
 * with a subtle opacity so the disabled state still telegraphs.
 */
export function SignInPill({
  spec,
  status,
  onClick,
  label,
}: {
  spec: OauthSpec;
  status: 'idle' | 'awaiting' | 'error';
  onClick: () => void;
  label: string;
}) {
  const awaiting = status === 'awaiting';
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={awaiting}
      className={cn(
        'inline-flex items-center gap-2 self-start rounded-full bg-foreground px-4 py-1.5 text-[13px] font-medium text-background',
        'shadow-sm ring-1 ring-shade/10 transition-all',
        'hover:bg-foreground/90 hover:shadow-md',
        'disabled:cursor-wait disabled:opacity-70 disabled:hover:bg-white disabled:hover:shadow-sm',
      )}
    >
      <img
        src={spec.icon}
        alt=""
        aria-hidden="true"
        draggable={false}
        className="size-4 shrink-0 rounded-sm object-contain"
      />
      {awaiting ? 'Waiting for browser…' : label}
    </button>
  );
}
