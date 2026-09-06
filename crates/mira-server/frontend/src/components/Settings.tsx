import { useEffect, useMemo, useState } from 'react';
import { getSettings, putSettings } from '../api';
import type { Mode, SettingsView } from '../types';

const PROVIDER_PRESETS = [
  { name: 'openrouter', base_url: 'https://openrouter.ai/api/v1', suggested_model: 'google/gemini-2.5-flash' },
  { name: 'openai',     base_url: 'https://api.openai.com/v1',    suggested_model: 'gpt-4o-mini' },
  { name: 'anthropic',  base_url: 'https://api.anthropic.com/v1', suggested_model: 'claude-sonnet-4-5' },
  { name: 'groq',       base_url: 'https://api.groq.com/openai/v1', suggested_model: 'moonshotai/kimi-k2-instruct' },
  { name: 'ollama',     base_url: 'http://localhost:11434/v1',    suggested_model: 'llama3.1' },
];

const MODES: Mode[] = ['plan', 'manual', 'auto', 'edit', 'yolo'];

type Props = {
  open: boolean;
  onClose: () => void;
  onSaved: (v: SettingsView) => void;
};

export function SettingsPanel({ open, onClose, onSaved }: Props) {
  const [view, setView] = useState<SettingsView | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [providerName, setProviderName] = useState<string>('openrouter');
  const [baseUrl, setBaseUrl] = useState<string>('');
  const [apiKey, setApiKey] = useState<string>('');
  const [showReplaceKey, setShowReplaceKey] = useState<boolean>(false);
  const [model, setModel] = useState<string>('');
  const [mode, setMode] = useState<Mode>('manual');
  const [maxTokens, setMaxTokens] = useState<string>('');

  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setLoadError(null);
    getSettings()
      .then((v) => {
        setView(v);
        hydrate(v);
      })
      .catch((e) => setLoadError(String(e.message ?? e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  function hydrate(v: SettingsView) {
    const pName = v.default_provider ?? 'openrouter';
    setProviderName(pName);
    const p = v.providers.find((x) => x.name === pName);
    const preset = PROVIDER_PRESETS.find((x) => x.name === pName);
    setBaseUrl(p?.base_url ?? preset?.base_url ?? '');
    setApiKey('');
    setShowReplaceKey(!(p?.has_api_key || p?.api_key_env));
    setModel(v.default_model ?? '');
    setMode((v.default_mode as Mode) ?? 'manual');
    setMaxTokens(v.max_tokens?.toString() ?? '');
  }

  const presetForName = useMemo(
    () => PROVIDER_PRESETS.find((x) => x.name === providerName),
    [providerName],
  );

  const currentProvider = useMemo(
    () => view?.providers.find((x) => x.name === providerName),
    [view, providerName],
  );

  const keyStatus: KeyStatus = useMemo(() => {
    if (!currentProvider) return { kind: 'none' };
    if (currentProvider.has_api_key && currentProvider.api_key_masked) {
      return { kind: 'literal', masked: currentProvider.api_key_masked };
    }
    if (currentProvider.api_key_env) {
      return { kind: 'env', name: currentProvider.api_key_env };
    }
    return { kind: 'none' };
  }, [currentProvider]);

  function onProviderChange(next: string) {
    setProviderName(next);
    const preset = PROVIDER_PRESETS.find((x) => x.name === next);
    const stored = view?.providers.find((x) => x.name === next);
    setBaseUrl(stored?.base_url ?? preset?.base_url ?? '');
    setApiKey('');
    setShowReplaceKey(!(stored?.has_api_key || stored?.api_key_env));
    if (preset && (!model || !stored?.has_api_key)) setModel(preset.suggested_model);
  }

  async function onSave() {
    setSaving(true);
    setSaveError(null);
    try {
      const parsedMax = maxTokens.trim() ? Number(maxTokens.trim()) : NaN;

      // Explicit null when the user cleared a field. `undefined` never
      // appears on the wire — that's reserved for api_key, which we omit
      // when the user didn't type a new one.
      const v = await putSettings({
        default_provider: providerName || null,
        default_model: model.trim() ? model.trim() : null,
        default_mode: mode,
        max_tokens: Number.isFinite(parsedMax) ? parsedMax : null,
        providers: [
          {
            name: providerName,
            base_url: baseUrl.trim() || null,
            // Send api_key only when the user actually typed a new one, or
            // asked to clear the stored one via the Replace toggle.
            api_key: apiKey.length > 0 ? apiKey : undefined,
          },
        ],
      });
      setView(v);
      onSaved(v);
      onClose();
    } catch (e: unknown) {
      setSaveError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  if (!open) return null;
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <div className="modal-title">Settings</div>
          <button className="link" onClick={onClose} aria-label="Close">✕</button>
        </div>

        {loadError && <div className="error">error: {loadError}</div>}
        {!view && !loadError && <div className="meta">loading…</div>}

        {view && (
          <div className="form">
            <label className="field">
              <span>Provider</span>
              <select value={providerName} onChange={(e) => onProviderChange(e.target.value)}>
                {PROVIDER_PRESETS.map((p) => (
                  <option key={p.name} value={p.name}>{p.name}</option>
                ))}
              </select>
            </label>

            <label className="field">
              <span>Base URL</span>
              <input
                type="text"
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                placeholder={presetForName?.base_url}
                spellCheck={false}
              />
            </label>

            <ApiKeyField
              status={keyStatus}
              showInput={showReplaceKey}
              value={apiKey}
              onChange={setApiKey}
              onReplace={() => { setShowReplaceKey(true); setApiKey(''); }}
              onCancelReplace={() => { setShowReplaceKey(false); setApiKey(''); }}
            />
            <div className="hint">
              Stored in <code>{view.config_path}</code>.
            </div>

            <label className="field">
              <span>Model</span>
              <input
                type="text"
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder={presetForName?.suggested_model}
                spellCheck={false}
              />
            </label>

            <label className="field">
              <span>Default mode</span>
              <select value={mode} onChange={(e) => setMode(e.target.value as Mode)}>
                {MODES.map((m) => (
                  <option key={m} value={m}>{m}</option>
                ))}
              </select>
            </label>

            <label className="field">
              <span>Max tokens</span>
              <input
                type="number"
                value={maxTokens}
                onChange={(e) => setMaxTokens(e.target.value)}
                min={1}
                placeholder="(provider default)"
              />
            </label>

            {saveError && <div className="error">error: {saveError}</div>}

            <div className="modal-actions">
              <button className="secondary" onClick={onClose} disabled={saving}>Cancel</button>
              <button className="primary" onClick={onSave} disabled={saving}>
                {saving ? 'Saving…' : 'Save'}
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

type KeyStatus =
  | { kind: 'none' }
  | { kind: 'literal'; masked: string }
  | { kind: 'env'; name: string };

function ApiKeyField({
  status, showInput, value, onChange, onReplace, onCancelReplace,
}: {
  status: KeyStatus;
  showInput: boolean;
  value: string;
  onChange: (v: string) => void;
  onReplace: () => void;
  onCancelReplace: () => void;
}) {
  return (
    <>
      <div className="field">
        <span>API key</span>
        <div className="key-cell">
          {status.kind === 'literal' && !showInput && (
            <div className="key-saved">
              <span className="key-check">✓</span>
              <span className="key-label">Key saved</span>
              <span className="key-masked">{status.masked}</span>
              <button type="button" className="key-replace" onClick={onReplace}>Replace</button>
            </div>
          )}
          {status.kind === 'env' && !showInput && (
            <div className="key-saved env">
              <span className="key-check">✓</span>
              <span className="key-label">From env</span>
              <span className="key-masked">${status.name}</span>
              <button type="button" className="key-replace" onClick={onReplace}>Override</button>
            </div>
          )}
          {(status.kind === 'none' || showInput) && (
            <div className="key-input-row">
              <input
                type="password"
                value={value}
                onChange={(e) => onChange(e.target.value)}
                placeholder={status.kind === 'literal' ? `replaces ${status.masked}` : 'sk-…'}
                autoComplete="off"
                spellCheck={false}
                autoFocus={showInput}
              />
              {status.kind !== 'none' && (
                <button type="button" className="key-cancel" onClick={onCancelReplace}>Cancel</button>
              )}
            </div>
          )}
        </div>
      </div>
    </>
  );
}
