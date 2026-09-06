import { useEffect, useMemo, useState } from 'react';
import { Check, Key } from '@phosphor-icons/react';
import { getSettings, putSettings } from '../api';
import type { Mode, SettingsView } from '../types';
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

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
      .then((v) => { setView(v); hydrate(v); })
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
      const v = await putSettings({
        default_provider: providerName || null,
        default_model: model.trim() ? model.trim() : null,
        default_mode: mode,
        max_tokens: Number.isFinite(parsedMax) ? parsedMax : null,
        providers: [{
          name: providerName,
          base_url: baseUrl.trim() || null,
          api_key: apiKey.length > 0 ? apiKey : undefined,
        }],
      });
      setView(v);
      onSaved(v);
      onClose();
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Settings</DialogTitle>
        </DialogHeader>

        {loadError && <div className="text-xs text-destructive font-mono">error: {loadError}</div>}
        {!view && !loadError && <div className="text-xs text-muted-foreground">loading…</div>}

        {view && (
          <div className="flex flex-col gap-3">
            <Field label="Provider">
              <select
                value={providerName}
                onChange={(e) => onProviderChange(e.target.value)}
                className="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus:ring-1 focus:ring-ring"
              >
                {PROVIDER_PRESETS.map((p) => <option key={p.name} value={p.name}>{p.name}</option>)}
              </select>
            </Field>

            <Field label="Base URL">
              <Input
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                placeholder={presetForName?.base_url}
                spellCheck={false}
              />
            </Field>

            <ApiKeyField
              status={keyStatus}
              showInput={showReplaceKey}
              value={apiKey}
              onChange={setApiKey}
              onReplace={() => { setShowReplaceKey(true); setApiKey(''); }}
              onCancelReplace={() => { setShowReplaceKey(false); setApiKey(''); }}
            />
            <p className="text-[11.5px] text-muted-foreground ml-[8.75rem] -mt-1.5">
              Stored in <code className="rounded bg-background px-1">{view.config_path}</code>.
            </p>

            <Field label="Model">
              <Input
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder={presetForName?.suggested_model}
                spellCheck={false}
              />
            </Field>

            <Field label="Default mode">
              <select
                value={mode}
                onChange={(e) => setMode(e.target.value as Mode)}
                className="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus:ring-1 focus:ring-ring"
              >
                {MODES.map((m) => <option key={m} value={m}>{m}</option>)}
              </select>
            </Field>

            <Field label="Max tokens">
              <Input
                type="number"
                value={maxTokens}
                onChange={(e) => setMaxTokens(e.target.value)}
                min={1}
                placeholder="(provider default)"
              />
            </Field>

            {saveError && <div className="text-xs text-destructive font-mono">error: {saveError}</div>}

            <DialogFooter>
              <Button variant="outline" onClick={onClose} disabled={saving}>Cancel</Button>
              <Button onClick={onSave} disabled={saving}>{saving ? 'Saving…' : 'Save'}</Button>
            </DialogFooter>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}

type KeyStatus =
  | { kind: 'none' }
  | { kind: 'literal'; masked: string }
  | { kind: 'env'; name: string };

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="grid grid-cols-[8rem_1fr] items-center gap-3">
      <span className="text-[12.5px] text-muted-foreground">{label}</span>
      {children}
    </label>
  );
}

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
    <div className="grid grid-cols-[8rem_1fr] items-center gap-3">
      <span className="text-[12.5px] text-muted-foreground">API key</span>
      <div className="min-w-0">
        {status.kind === 'literal' && !showInput && (
          <SavedBadge tone="green" icon={<Check className="size-3.5" />} label="Key saved" masked={status.masked} onClick={onReplace} action="Replace" />
        )}
        {status.kind === 'env' && !showInput && (
          <SavedBadge tone="blue" icon={<Key className="size-3.5" />} label="From env" masked={`$${status.name}`} onClick={onReplace} action="Override" />
        )}
        {(status.kind === 'none' || showInput) && (
          <div className="flex gap-2">
            <Input
              type="password"
              value={value}
              onChange={(e) => onChange(e.target.value)}
              placeholder={status.kind === 'literal' ? `replaces ${status.masked}` : 'sk-…'}
              autoComplete="off"
              spellCheck={false}
              autoFocus={showInput}
            />
            {status.kind !== 'none' && (
              <Button variant="outline" size="sm" onClick={onCancelReplace} type="button">Cancel</Button>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function SavedBadge({
  tone, icon, label, masked, onClick, action,
}: {
  tone: 'green' | 'blue';
  icon: React.ReactNode;
  label: string;
  masked: string;
  onClick: () => void;
  action: string;
}) {
  return (
    <div
      className={cn(
        'flex items-center gap-2 rounded-md border px-2.5 py-1.5 text-[12.5px]',
        tone === 'green'
          ? 'border-[#98c379]/40 bg-[#98c379]/[0.06]'
          : 'border-mira-blue/40 bg-mira-blue/[0.06]',
      )}
    >
      <span className={tone === 'green' ? 'text-[#98c379]' : 'text-mira-blue'}>{icon}</span>
      <span className="text-muted-foreground">{label}</span>
      <span className="flex-1 min-w-0 font-mono text-xs overflow-hidden text-ellipsis whitespace-nowrap">{masked}</span>
      <Button variant="outline" size="sm" onClick={onClick} type="button">{action}</Button>
    </div>
  );
}
