import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { useEffect, useMemo, useState } from 'react';
import { Outlet } from 'react-router';
import { getSettings, putSettings } from '../api';
import { PROVIDER_PRESETS } from '../lib/providers';
import type { KeyUpdate, MemoryUpdate, Mode, ProviderUpdate, SettingsView } from '../types';
import { ErrorNotice } from './ErrorNotice';
import { memoryPatchFor } from './settings/MemorySection';
import { SETTINGS_SECTIONS } from './settings/sections';
import { useSettingsNavigation, type SettingsDraftContext } from './settings/SettingsContext';
import { Draft, EMPTY_DRAFT, EMPTY_MEMORY } from './settings/types';
// Local alias — the exported name is `SETTINGS_SECTIONS` (used by the
// Sidebar); everywhere inside this file we still refer to it as
// `SECTIONS` to keep the diff tight.
const SECTIONS = SETTINGS_SECTIONS;

export function SettingsSurface() {
  const navigation = useSettingsNavigation();
  const { section, onSaved, onExit } = navigation;
  const [view, setView] = useState<SettingsView | null>(null);
  const [loadAttempt, setLoadAttempt] = useState(0);
  const [loadingSettings, setLoadingSettings] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [draft, setDraft] = useState<Draft>(EMPTY_DRAFT);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  // Refetch settings from the server without re-hydrating the draft.
  // Used after an OAuth sign-in flow lands a new key server-side —
  // the UI needs to see `has_api_key: true` without clobbering any
  // in-progress edits.
  const refetch = () => {
    getSettings()
      .then((v) => {
        setView(v);
        // Only rehydrate the API-key visibility flag; keep other draft
        // fields untouched so a mid-edit doesn't get reset.
        const p = v.providers.find((x) => x.name === draft.providerName);
        setDraft((d) => ({
          ...d,
          showReplaceKey: !(p?.has_api_key || p?.api_key_env),
        }));
        onSaved(v);
      })
      .catch(() => {
        /* leave stale view; user can retry */
      });
  };

  // Load once on mount. The surface persists across section swaps
  // (the sidebar changes `section` without unmounting us), so we
  // shouldn't re-hydrate on every section click — that would drop
  // in-progress edits when the user navigates away and back.
  useEffect(() => {
    let cancelled = false;
    setLoadingSettings(true);
    getSettings()
      .then((v) => {
        if (!cancelled) {
          setView(v);
          hydrate(v);
          setLoadError(null);
        }
      })
      .catch((e) => {
        if (!cancelled) setLoadError(String(e.message ?? e));
      })
      .finally(() => {
        if (!cancelled) setLoadingSettings(false);
      });
    return () => {
      cancelled = true;
    };
    // Only hydrate an initial successful load; retry is offered for failed loads.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadAttempt]);

  function hydrate(v: SettingsView) {
    const pName = v.default_provider ?? 'openrouter';
    const p = v.providers.find((x) => x.name === pName);
    const preset = PROVIDER_PRESETS.find((x) => x.name === pName);
    setDraft({
      providerName: pName,
      baseUrl: p?.base_url ?? preset?.base_url ?? '',
      apiKey: '',
      showReplaceKey: !(p?.has_api_key || p?.api_key_env),
      model: v.default_model ?? '',
      smallModel: v.small_model ?? '',
      mode: (v.default_mode as Mode) ?? 'manual',
      maxTokens: v.max_tokens?.toString() ?? '',
      keyValues: {},
      keyEditing: {},
      memory: v.memory ?? EMPTY_MEMORY,
    });
  }

  async function onSave() {
    setSaving(true);
    setSaveError(null);
    try {
      const parsedMax = draft.maxTokens.trim() ? Number(draft.maxTokens.trim()) : NaN;

      const providers: ProviderUpdate[] = [
        {
          name: draft.providerName,
          base_url: draft.baseUrl.trim() || null,
          api_key: draft.apiKey.length > 0 ? draft.apiKey : undefined,
        },
      ];

      const keys: KeyUpdate[] = Object.entries(draft.keyValues)
        .filter(([, v]) => v !== undefined)
        .map(([name, value]) => ({ name, value }));

      // Only include a memory patch when something actually changed —
      // otherwise we'd stamp the yaml with the effective defaults every
      // save, which loses the "field absent, use default" semantics.
      const memPatch: MemoryUpdate | undefined = memoryPatchFor(view!.memory, draft.memory);

      const v = await putSettings({
        default_provider: draft.providerName || null,
        default_model: draft.model.trim() ? draft.model.trim() : null,
        small_model: draft.smallModel.trim() ? draft.smallModel.trim() : null,
        default_mode: draft.mode,
        max_tokens: Number.isFinite(parsedMax) ? parsedMax : null,
        providers,
        keys,
        ...(memPatch ? { memory: memPatch } : {}),
      });
      setView(v);
      onSaved(v);
      // Stay in settings — the sidebar handles navigation now.
      // Users click "Back to App" (or another sidebar item) to leave.
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  const dirty = useMemo(() => {
    if (!view) return false;
    if (draft.apiKey.length > 0) return true;
    if (Object.values(draft.keyValues).some((v) => v !== undefined)) return true;
    if (draft.providerName !== (view.default_provider ?? 'openrouter')) return true;
    if (draft.model !== (view.default_model ?? '')) return true;
    if (draft.smallModel !== (view.small_model ?? '')) return true;
    if (draft.mode !== (view.default_mode ?? 'manual')) return true;
    if (draft.maxTokens !== (view.max_tokens?.toString() ?? '')) return true;
    const p = view.providers.find((x) => x.name === draft.providerName);
    const preset = PROVIDER_PRESETS.find((x) => x.name === draft.providerName);
    if (draft.baseUrl !== (p?.base_url ?? preset?.base_url ?? '')) return true;
    if (memoryPatchFor(view.memory, draft.memory) !== undefined) return true;
    return false;
  }, [draft, view]);

  const currentLabel = SECTIONS.find((s) => s.id === section)?.label ?? 'Settings';

  return (
    // min-h-0 + overflow-hidden: without them this flex child's automatic
    // min-height is its content height, so a long section (usage, skills)
    // stretches the surface past the viewport and the whole app scrolls
    // — body scroll moves the sidebar too. With them the surface is
    // locked to the main pane and only the content area below scrolls.
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      {/* Top bar mirrors the chat/other-view header height so the layout
          doesn't shift when the user enters settings. */}
      <div className="flex h-11 shrink-0 items-center gap-3 border-b border-border/60 px-4">
        <span className="min-w-0 flex-1 truncate text-[13.5px] text-foreground">
          {currentLabel}
        </span>
        {view && (
          <div
            className={cn(
              'inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[10.5px]',
              view.configured
                ? 'bg-emerald-500/10 text-emerald-400'
                : 'bg-amber-500/10 text-amber-300',
            )}
          >
            <span
              className={cn(
                'size-1.5 rounded-full',
                view.configured ? 'bg-emerald-500' : 'bg-amber-500',
              )}
            />
            {view.configured ? 'Configured' : 'Needs API key'}
          </div>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        <div
          key={section}
          className={cn(
            'mx-auto w-full animate-fade-in px-6 py-6',
            section === 'usage'
              ? 'max-w-3xl'
              : section === 'hooks' ||
                  section === 'keybindings' ||
                  section === 'skills' ||
                  section === 'agents' ||
                  section === 'subagents'
                ? 'max-w-5xl'
                : 'max-w-2xl',
          )}
        >
          {loadError && (
            <ErrorNotice
              title="Couldn't load settings"
              description="Check that Mira is running, then try again. Local appearance preferences are still available."
              details={loadError}
              pending={loadingSettings}
              onRetry={() => setLoadAttempt((n) => n + 1)}
            />
          )}
          {saveError && (
            <ErrorNotice
              title="Couldn't save settings"
              description="Your edits are still here. Use Retry save below to try again."
              details={saveError}
            />
          )}
          {!view && !loadError && (
            <div className="text-[12.5px] text-muted-foreground">loading…</div>
          )}

          <Outlet
            context={
              { ...navigation, view, draft, setDraft, refetch } satisfies SettingsDraftContext
            }
          />
        </div>
      </div>

      {view && (
        <div className="flex shrink-0 items-center justify-between border-t border-border/60 px-6 py-3">
          <div className="text-[11.5px] text-muted-foreground">
            {saveError ? (
              <span className="text-destructive">Changes not saved</span>
            ) : dirty ? (
              'Unsaved changes'
            ) : (
              'Up to date'
            )}
          </div>
          <div className="flex items-center gap-2">
            <Button variant="outline" onClick={onExit} disabled={saving}>
              Back to app
            </Button>
            <Button onClick={onSave} disabled={saving || !dirty}>
              {saving ? 'Saving…' : saveError ? 'Retry save' : 'Save'}
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}

export type { GithubReturn } from './Integrations';
export { SETTINGS_SECTIONS, type SettingsSectionId } from './settings/sections';
