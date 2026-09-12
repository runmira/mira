import { useEffect, useMemo, useState } from 'react';
import {
  ArrowClockwise,
  Book,
  BookOpen,
  BracketsCurly,
  Brain,
  Bug,
  Check,
  Code,
  Database,
  Eye,
  FileText,
  FolderOpen,
  Gear,
  GitBranch,
  GitCommit,
  GitMerge,
  GitPullRequest,
  Info,
  Key,
  Lightbulb,
  MagnifyingGlass,
  NotePencil,
  Package,
  Paperclip,
  Plug,
  Rocket,
  ScanSmiley,
  Shield,
  ShieldCheck,
  Sliders,
  Sparkle,
  Target,
  Terminal,
  Wrench,
  X,
} from '@phosphor-icons/react';
import { getSettings, listSkills, putSettings, reloadSkills, type SkillView } from '../api';
import type {
  KeyUpdate,
  MemoryUpdate,
  MemoryView,
  Mode,
  ProviderUpdate,
  SettingsView,
} from '../types';
import { Dialog, DialogContent } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

/**
 * Two-pane settings: sidebar of sections on the left, the active section's
 * fields on the right. State is pooled at this level so every section
 * edits the same `draft` and a single "Save" commits everything at once —
 * users can jump between sections without losing work.
 */

const PROVIDER_PRESETS = [
  { name: 'openrouter', base_url: 'https://openrouter.ai/api/v1', suggested_model: 'google/gemini-2.5-flash' },
  { name: 'openai',     base_url: 'https://api.openai.com/v1',    suggested_model: 'gpt-4o-mini' },
  { name: 'anthropic',  base_url: 'https://api.anthropic.com/v1', suggested_model: 'claude-sonnet-4-5' },
  { name: 'groq',       base_url: 'https://api.groq.com/openai/v1', suggested_model: 'moonshotai/kimi-k2-instruct' },
  { name: 'ollama',     base_url: 'http://localhost:11434/v1',    suggested_model: 'llama3.1' },
];

const MODES: Mode[] = ['plan', 'manual', 'auto', 'edit', 'yolo'];

const MODE_DESCRIPTIONS: Record<Mode, string> = {
  plan: 'Plan first, then execute with approvals.',
  manual: 'Ask before every write, edit, or command.',
  auto: 'Auto-approve writes/edits; ask on bash.',
  edit: 'Auto-approve everything unless a rule blocks.',
  yolo: 'No gating — do whatever the model asks.',
};

/** Well-known third-party keys we know how to help configure. Extend as
 *  we add tools; anything not here still appears if the backend surfaces
 *  it — this list just adds friendly labels + signup links. */
const KEY_META: Record<string, { label: string; help: string; url?: string }> = {
  BRAVE_SEARCH_API_KEY: {
    label: 'Brave Search',
    help: 'Powers the `web_search` tool. 2000 queries/month free tier.',
    url: 'https://api.search.brave.com/',
  },
  TAVILY_API_KEY: {
    label: 'Tavily',
    help: 'Alternative search backend (not yet wired). Free tier.',
    url: 'https://tavily.com/',
  },
  GITHUB_TOKEN: {
    label: 'GitHub',
    help: 'Powers the pull-request panel. The link opens a new-token page with the `repo` scope pre-selected — enough to list, comment on, review, and merge PRs.',
    url: 'https://github.com/settings/tokens/new?scopes=repo&description=Mira',
  },
};

type SectionId = 'provider' | 'preferences' | 'memory' | 'skills' | 'search' | 'about';

const SECTIONS: { id: SectionId; label: string; icon: React.ComponentType<{ className?: string }> }[] = [
  { id: 'provider',    label: 'Provider',    icon: Plug },
  { id: 'preferences', label: 'Preferences', icon: Sliders },
  { id: 'memory',      label: 'Memory',      icon: Brain },
  { id: 'skills',      label: 'Skills',      icon: Sparkle },
  { id: 'search',      label: 'Search & keys', icon: MagnifyingGlass },
  { id: 'about',       label: 'About',       icon: Info },
];

type Props = {
  open: boolean;
  onClose: () => void;
  onSaved: (v: SettingsView) => void;
};

type Draft = {
  providerName: string;
  baseUrl: string;
  apiKey: string;
  showReplaceKey: boolean;
  model: string;
  mode: Mode;
  maxTokens: string;
  keyValues: Record<string, string>; // key name → new pending value
  keyEditing: Record<string, boolean>; // which keys are in edit mode
  memory: MemoryView;
};

const EMPTY_MEMORY: MemoryView = {
  auto_extract: true,
  tools_enabled: true,
  inject_context: true,
  extractor_model: null,
};

const EMPTY_DRAFT: Draft = {
  providerName: 'openrouter',
  baseUrl: '',
  apiKey: '',
  showReplaceKey: false,
  model: '',
  mode: 'manual',
  maxTokens: '',
  keyValues: {},
  keyEditing: {},
  memory: EMPTY_MEMORY,
};

export function SettingsPanel({ open, onClose, onSaved }: Props) {
  const [view, setView] = useState<SettingsView | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [draft, setDraft] = useState<Draft>(EMPTY_DRAFT);
  const [section, setSection] = useState<SectionId>('provider');
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setLoadError(null);
    setSaveError(null);
    setSection('provider');
    getSettings()
      .then((v) => { setView(v); hydrate(v); })
      .catch((e) => setLoadError(String(e.message ?? e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

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

      const providers: ProviderUpdate[] = [{
        name: draft.providerName,
        base_url: draft.baseUrl.trim() || null,
        api_key: draft.apiKey.length > 0 ? draft.apiKey : undefined,
      }];

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
        default_mode: draft.mode,
        max_tokens: Number.isFinite(parsedMax) ? parsedMax : null,
        providers,
        keys,
        ...(memPatch ? { memory: memPatch } : {}),
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

  const dirty = useMemo(() => {
    if (!view) return false;
    if (draft.apiKey.length > 0) return true;
    if (Object.values(draft.keyValues).some((v) => v !== undefined)) return true;
    if (draft.providerName !== (view.default_provider ?? 'openrouter')) return true;
    if (draft.model !== (view.default_model ?? '')) return true;
    if (draft.mode !== (view.default_mode ?? 'manual')) return true;
    if (draft.maxTokens !== (view.max_tokens?.toString() ?? '')) return true;
    const p = view.providers.find((x) => x.name === draft.providerName);
    const preset = PROVIDER_PRESETS.find((x) => x.name === draft.providerName);
    if (draft.baseUrl !== (p?.base_url ?? preset?.base_url ?? '')) return true;
    if (memoryPatchFor(view.memory, draft.memory) !== undefined) return true;
    return false;
  }, [draft, view]);

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-3xl p-0 gap-0 overflow-hidden">
        <div className="flex h-[560px] max-h-[80vh]">
          {/* --- sidebar --- */}
          <aside className="flex w-[180px] shrink-0 flex-col border-r border-border/60 bg-secondary/30">
            <div className="flex items-center justify-between px-4 pt-4 pb-3">
              <span className="text-[15px] font-semibold tracking-tight">Settings</span>
              <button
                type="button"
                onClick={onClose}
                className="rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
                aria-label="Close"
              >
                <X className="size-4" />
              </button>
            </div>
            <nav className="flex flex-col gap-0.5 px-2 py-2">
              {SECTIONS.map((s) => {
                const active = section === s.id;
                const Icon = s.icon;
                return (
                  <button
                    key={s.id}
                    type="button"
                    onClick={() => setSection(s.id)}
                    className={cn(
                      'flex items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-[13.5px] transition-colors',
                      active
                        ? 'bg-accent text-foreground'
                        : 'text-foreground/80 hover:bg-accent/60 hover:text-foreground',
                    )}
                  >
                    <Icon className={cn('size-4 shrink-0', active ? 'text-mira-blue' : 'text-muted-foreground')} />
                    <span>{s.label}</span>
                  </button>
                );
              })}
            </nav>
            <div className="flex-1" />
            {view && (
              <div className="px-3 pb-3 text-[10.5px] text-muted-foreground/70">
                <div
                  className={cn(
                    'inline-flex items-center gap-1.5 rounded-full px-2 py-0.5',
                    view.configured
                      ? 'bg-emerald-500/10 text-emerald-400'
                      : 'bg-amber-500/10 text-amber-300',
                  )}
                >
                  <span className={cn('size-1.5 rounded-full', view.configured ? 'bg-emerald-500' : 'bg-amber-500')} />
                  {view.configured ? 'Configured' : 'Needs API key'}
                </div>
              </div>
            )}
          </aside>

          {/* --- active section --- */}
          <div className="flex min-w-0 flex-1 flex-col">
            <div className="flex-1 overflow-y-auto px-6 py-5">
              {loadError && (
                <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-[12.5px] text-destructive">
                  error: {loadError}
                </div>
              )}
              {!view && !loadError && (
                <div className="text-[12.5px] text-muted-foreground">loading…</div>
              )}

              {view && section === 'provider' && (
                <ProviderSection view={view} draft={draft} setDraft={setDraft} />
              )}
              {view && section === 'preferences' && (
                <PreferencesSection draft={draft} setDraft={setDraft} />
              )}
              {view && section === 'memory' && (
                <MemorySection draft={draft} setDraft={setDraft} />
              )}
              {view && section === 'skills' && (
                <SkillsSection />
              )}
              {view && section === 'search' && (
                <KeysSection view={view} draft={draft} setDraft={setDraft} />
              )}
              {view && section === 'about' && (
                <AboutSection view={view} />
              )}
            </div>

            {view && (
              <div className="flex items-center justify-between border-t border-border/60 px-6 py-3">
                <div className="text-[11.5px] text-muted-foreground">
                  {saveError ? <span className="text-destructive">{saveError}</span>
                    : dirty ? 'Unsaved changes'
                    : 'Up to date'}
                </div>
                <div className="flex items-center gap-2">
                  <Button variant="outline" onClick={onClose} disabled={saving}>Cancel</Button>
                  <Button onClick={onSave} disabled={saving || !dirty}>
                    {saving ? 'Saving…' : 'Save'}
                  </Button>
                </div>
              </div>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/* ---------- section: provider ---------- */

function ProviderSection({
  view, draft, setDraft,
}: {
  view: SettingsView;
  draft: Draft;
  setDraft: (u: (d: Draft) => Draft) => void;
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
    <SectionShell
      title="Model provider"
      subtitle="Where Mira sends chat requests. All providers speak the OpenAI-compatible /chat/completions wire."
    >
      <Field label="Provider" hint="Preset endpoints — you can override the base URL below.">
        <select
          value={draft.providerName}
          onChange={(e) => onProviderChange(e.target.value)}
          className="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus:ring-1 focus:ring-ring"
        >
          {PROVIDER_PRESETS.map((p) => <option key={p.name} value={p.name}>{p.name}</option>)}
        </select>
      </Field>

      <Field label="Base URL">
        <Input
          value={draft.baseUrl}
          onChange={(e) => setDraft((d) => ({ ...d, baseUrl: e.target.value }))}
          placeholder={preset?.base_url}
          spellCheck={false}
        />
      </Field>

      <ApiKeyField
        status={keyStatus}
        showInput={draft.showReplaceKey}
        value={draft.apiKey}
        onChange={(v) => setDraft((d) => ({ ...d, apiKey: v }))}
        onReplace={() => setDraft((d) => ({ ...d, showReplaceKey: true, apiKey: '' }))}
        onCancelReplace={() => setDraft((d) => ({ ...d, showReplaceKey: false, apiKey: '' }))}
      />

      <Field label="Model" hint="The specific model id sent with each request.">
        <Input
          value={draft.model}
          onChange={(e) => setDraft((d) => ({ ...d, model: e.target.value }))}
          placeholder={preset?.suggested_model}
          spellCheck={false}
        />
      </Field>
    </SectionShell>
  );
}

/* ---------- section: preferences ---------- */

function PreferencesSection({
  draft, setDraft,
}: {
  draft: Draft;
  setDraft: (u: (d: Draft) => Draft) => void;
}) {
  return (
    <SectionShell
      title="Preferences"
      subtitle="Defaults new sessions inherit. Override per-session via the composer chips."
    >
      <Field label="Default mode" hint={MODE_DESCRIPTIONS[draft.mode]}>
        <select
          value={draft.mode}
          onChange={(e) => setDraft((d) => ({ ...d, mode: e.target.value as Mode }))}
          className="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus:ring-1 focus:ring-ring"
        >
          {MODES.map((m) => <option key={m} value={m}>{m}</option>)}
        </select>
      </Field>

      <Field
        label="Max tokens"
        hint="Cap on tokens the model can emit per response. Leave blank for the provider default."
      >
        <Input
          type="number"
          value={draft.maxTokens}
          onChange={(e) => setDraft((d) => ({ ...d, maxTokens: e.target.value }))}
          min={1}
          placeholder="(provider default)"
        />
      </Field>
    </SectionShell>
  );
}

/* ---------- section: memory ---------- */

function MemorySection({
  draft, setDraft,
}: {
  draft: Draft;
  setDraft: (u: (d: Draft) => Draft) => void;
}) {
  function update<K extends keyof MemoryView>(key: K, value: MemoryView[K]) {
    setDraft((d) => ({ ...d, memory: { ...d.memory, [key]: value } }));
  }

  return (
    <SectionShell
      title="Memory"
      subtitle="Cross-session memory: user + project MIRA.md, plus the agent-written episodic stream at .mira/episodic.jsonl. Changes apply to new chats — click New chat after saving to try them."
    >
      <ToggleField
        label="Inject memory into prompt"
        hint="Add a live 'memory' section to every model request. Turn off to shrink the system prompt back to the pre-memory baseline — useful for isolating whether the injected content is confusing the model."
        checked={draft.memory.inject_context}
        onChange={(v) => update('inject_context', v)}
      />

      <ToggleField
        label="Enable memory tools"
        hint="Registers memory_read / memory_search / memory_append / memory_edit / memory_remember. Turn off to remove them from the model's tool list — useful when the extra tools distract simple questions."
        checked={draft.memory.tools_enabled}
        onChange={(v) => update('tools_enabled', v)}
      />

      <ToggleField
        label="Auto-extract facts after each turn"
        hint="Background pass that mines each finished turn for durable facts and appends them to .mira/episodic.jsonl. Only fires when at least one tool call succeeded."
        checked={draft.memory.auto_extract}
        onChange={(v) => update('auto_extract', v)}
      />

      <Field
        label="Extractor model"
        hint="Model id used for the extraction call. Leave blank to reuse the session's active model (works but is expensive). Point at your provider's cheap tier — e.g. claude-haiku-4-5, gpt-5-nano, deepseek-chat — for negligible per-round cost."
      >
        <Input
          value={draft.memory.extractor_model ?? ''}
          onChange={(e) => update('extractor_model', e.target.value || null)}
          placeholder="(uses session model)"
          spellCheck={false}
        />
      </Field>

      <div className="rounded-md border border-amber-500/30 bg-amber-500/[0.06] px-3 py-2 text-[11.5px] text-amber-300/90">
        Settings are saved to <code className="font-mono">mira.yaml</code>. To
        apply them to the running server, <b>restart</b> Mira (Ctrl+C then run
        the command again). Hot-reload without restart is not yet wired.
      </div>
    </SectionShell>
  );
}

/* ---------- skills ---------- */

/**
 * Loaded-skill browser. Card grid, each skill rendered as a rounded
 * card with a colored icon badge + name + description + metadata chips.
 * Colors + icons come from the skill's frontmatter (`color:` and
 * `icon:` fields); unknowns fall back to stable name-hash-derived tints
 * so user-added skills still look distinct even without opinions.
 *
 * Fetches `/api/skills` on mount; a "Reload from disk" button re-reads
 * `~/.mira/skills/` + `<cwd>/.mira/skills/` (via `POST /api/skills/reload`)
 * so newly-added skill files appear without a restart.
 *
 * Groups by tier (Bundled → User → Project). Each tier gets its own
 * header row with a count. Bundled skills always render first — they're
 * the "official" roster the user can rely on.
 */
function SkillsSection() {
  const [skills, setSkills] = useState<SkillView[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastReload, setLastReload] = useState<number | null>(null);

  useEffect(() => {
    setLoading(true);
    listSkills()
      .then((s) => { setSkills(s); setError(null); })
      .catch((e) => setError(String((e as Error).message)))
      .finally(() => setLoading(false));
  }, []);

  async function reload() {
    setLoading(true);
    setError(null);
    try {
      const fresh = await reloadSkills();
      setSkills(fresh);
      setLastReload(Date.now());
    } catch (e) {
      setError(String((e as Error).message));
    } finally {
      setLoading(false);
    }
  }

  const grouped = useMemo(() => {
    const out: Record<'bundled' | 'user' | 'project', SkillView[]> = {
      bundled: [],
      user: [],
      project: [],
    };
    for (const s of skills ?? []) {
      out[s.tier].push(s);
    }
    (['bundled', 'user', 'project'] as const).forEach((k) => {
      out[k].sort((a, b) => a.name.localeCompare(b.name));
    });
    return out;
  }, [skills]);

  const total = skills?.length ?? 0;

  return (
    <SectionShell
      title="Skills"
      subtitle="Reusable instruction bundles the agent invokes to accomplish a specific task. Add your own to ~/.mira/skills/ (user-wide) or <cwd>/.mira/skills/ (per-repo)."
    >
      {/* Header strip: reload button + status. Sits above the grid so
       *  it doesn't consume vertical space when there are many skills. */}
      <div className="flex items-center justify-between rounded-lg border border-border/50 bg-secondary/30 px-3 py-2">
        <div className="flex items-center gap-2">
          <span className="text-[13px] font-medium text-foreground">
            {loading ? 'Loading…' : `${total} skill${total === 1 ? '' : 's'} loaded`}
          </span>
          {lastReload && !loading && (
            <span className="text-[11.5px] text-muted-foreground">
              · reloaded {timeAgoSecs(lastReload)}
            </span>
          )}
        </div>
        <button
          type="button"
          onClick={reload}
          disabled={loading}
          className={cn(
            'inline-flex items-center gap-1.5 rounded-md border border-border/70 bg-background/60 px-2.5 py-1 text-[12px] font-medium text-foreground transition-colors',
            'hover:bg-background hover:border-border disabled:opacity-50 disabled:cursor-not-allowed',
          )}
          title="Re-read skill files from disk"
        >
          <ArrowClockwise className={cn('size-3.5', loading && 'animate-spin')} weight="bold" />
          Reload
        </button>
      </div>

      {error && (
        <div className="rounded-md border border-destructive/40 bg-destructive/[0.08] px-3 py-2 text-[12px] text-destructive">
          {error}
        </div>
      )}

      {!error && !loading && skills != null && total === 0 && (
        <EmptySkillsState />
      )}

      {(['bundled', 'user', 'project'] as const).map((tier) => {
        const entries = grouped[tier];
        if (entries.length === 0) return null;
        return (
          <div key={tier} className="flex flex-col gap-2.5">
            <div className="flex items-center gap-2 px-0.5">
              <span className={cn(
                'inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-[10.5px] font-semibold uppercase tracking-wider',
                tierChipClass(tier),
              )}>
                <span className={cn('size-1.5 rounded-full', tierDotClass(tier))} />
                {tierLabel(tier)}
              </span>
              <span className="text-[11.5px] text-muted-foreground">{entries.length}</span>
            </div>
            <div className="flex flex-col gap-2">
              {entries.map((s) => (
                <SkillCard key={`${tier}-${s.name}`} skill={s} />
              ))}
            </div>
          </div>
        );
      })}
    </SectionShell>
  );
}

/** One skill rendered as a card. Colored icon badge on the left,
 *  name/description stacked in the center, category + attachments chips
 *  underneath. Hover raises the card slightly to hint at future
 *  clickability (open the SKILL.md?), even though the current version
 *  is display-only. */
function SkillCard({ skill }: { skill: SkillView }) {
  const iconKey = skill.icon ?? defaultIconKey(skill);
  const Icon = iconFor(iconKey);
  const palette = paletteFor(skill.color, skill.name);

  return (
    <div
      // Card frame stays neutral (matches the "X skills loaded" strip
      // above) so the row list reads calmly. Colour lives only on the
      // icon square — it's the visual anchor, and lets the eye pick
      // out a skill by its accent without overwhelming the panel.
      className="group relative flex gap-3 rounded-lg border border-border/50 bg-secondary/30 p-3 transition-colors hover:border-border"
    >
      <div
        className={cn(
          'inline-flex size-9 shrink-0 items-center justify-center rounded-lg',
          palette.iconBg,
          palette.iconText,
        )}
      >
        <Icon weight="duotone" className="size-5" />
      </div>

      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex items-baseline gap-2">
          <span className="truncate font-mono text-[13.5px] font-semibold text-foreground">
            /{skill.name}
          </span>
          {skill.category && (
            <span className={cn(
              'shrink-0 rounded-sm px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wider',
              palette.chipBg,
              palette.chipText,
            )}>
              {skill.category}
            </span>
          )}
        </div>
        <p className="text-[12.5px] leading-snug text-foreground/80">
          {skill.description}
        </p>
        {skill.has_attachments && (
          <div className="mt-0.5 inline-flex items-center gap-1 text-[10.5px] text-muted-foreground/80">
            <Paperclip className="size-3" />
            attached resources
          </div>
        )}
      </div>
    </div>
  );
}

function EmptySkillsState() {
  return (
    <div className="flex flex-col items-center gap-3 rounded-xl border border-dashed border-border/50 bg-secondary/20 px-4 py-8 text-center">
      <div className="inline-flex size-10 items-center justify-center rounded-full bg-mira-blue/10 text-mira-blue">
        <Sparkle weight="duotone" className="size-5" />
      </div>
      <div>
        <div className="text-[13px] font-semibold text-foreground">No skills loaded</div>
        <div className="mt-1 text-[12px] text-muted-foreground">
          Drop a <code className="font-mono text-foreground/85">SKILL.md</code> into{' '}
          <code className="font-mono text-foreground/85">~/.mira/skills/&lt;name&gt;/</code>{' '}
          and hit Reload — or ask mira via <code className="font-mono text-foreground/85">/skill-creator</code>.
        </div>
      </div>
    </div>
  );
}

/* ---------- skill icon + color mapping ---------- */

/** Kebab-case icon name → Phosphor component. Curated so a user's
 *  frontmatter `icon: shield-check` picks up a matching component
 *  without every phosphor icon getting bundled. Unknown names fall
 *  through to Sparkle. */
function iconFor(name: string): React.ComponentType<{ className?: string; weight?: 'thin' | 'light' | 'regular' | 'bold' | 'fill' | 'duotone' }> {
  switch (name) {
    case 'shield-check':      return ShieldCheck;
    case 'shield':            return Shield;
    case 'magnifying-glass':  return MagnifyingGlass;
    case 'bug':               return Bug;
    case 'folder-open':       return FolderOpen;
    case 'file-text':
    case 'file':              return FileText;
    case 'git-branch':        return GitBranch;
    case 'git-commit':        return GitCommit;
    case 'git-merge':         return GitMerge;
    case 'git-pull-request':  return GitPullRequest;
    case 'lightbulb':         return Lightbulb;
    case 'target':            return Target;
    case 'gear':
    case 'settings':          return Gear;
    case 'wrench':            return Wrench;
    case 'rocket':            return Rocket;
    case 'package':           return Package;
    case 'database':          return Database;
    case 'terminal':          return Terminal;
    case 'code':              return Code;
    case 'brackets-curly':    return BracketsCurly;
    case 'book':              return Book;
    case 'book-open':         return BookOpen;
    case 'eye':               return Eye;
    case 'scan':              return ScanSmiley;
    case 'note-pencil':       return NotePencil;
    case 'sparkle':
    default:                  return Sparkle;
  }
}

/** Pick a default icon key for a skill that didn't specify one. Bundled
 *  skills always ship an explicit `icon:`; this only fires for user-
 *  added ones. We nudge by category — a `git`-category skill without an
 *  icon still reads sensibly as a git-branch. */
function defaultIconKey(s: SkillView): string {
  const cat = (s.category ?? '').toLowerCase();
  switch (cat) {
    case 'git':        return 'git-branch';
    case 'review':     return 'magnifying-glass';
    case 'qa':         return 'shield-check';
    case 'debug':      return 'bug';
    case 'meta':       return 'sparkle';
    case 'onboarding': return 'folder-open';
    case 'deploy':     return 'rocket';
    default:           return 'sparkle';
  }
}

type Palette = {
  iconBg: string;
  iconText: string;
  chipBg: string;
  chipText: string;
};

/** Color name → tailwind classes.
 *
 *  Each hue's classes are HARD-CODED (not template-interpolated)
 *  because Tailwind's JIT scanner only ships classes it can see as
 *  literal strings. `` `bg-${color}-500/15` `` compiles fine but the
 *  class never gets emitted → the card renders unstyled. Every hue
 *  below lists all six slots explicitly.
 *
 *  Unknown/missing color names hash by skill name to a stable wheel
 *  pick, so a user-added skill with `color: whatever` still gets a
 *  coherent card rather than a bland fallback. */
function paletteFor(name: string | undefined, fallbackSeed: string): Palette {
  const key = (name?.toLowerCase() ?? hashPick(fallbackSeed));
  const normalized =
    key === 'green'  ? 'emerald' :
    key === 'sky'    ? 'blue' :
    key === 'purple' ? 'violet' :
    key === 'rose'   ? 'pink' :
    key === 'yellow' ? 'amber' :
    key;
  switch (normalized) {
    case 'emerald':
      return {
        iconBg:     'bg-emerald-500/15',
        iconText:   'text-emerald-300',
        chipBg:     'bg-emerald-500/15',
        chipText:   'text-emerald-300',
      };
    case 'blue':
      return {
        iconBg:     'bg-blue-500/15',
        iconText:   'text-blue-300',
        chipBg:     'bg-blue-500/15',
        chipText:   'text-blue-300',
      };
    case 'indigo':
      return {
        iconBg:     'bg-indigo-500/15',
        iconText:   'text-indigo-300',
        chipBg:     'bg-indigo-500/15',
        chipText:   'text-indigo-300',
      };
    case 'violet':
      return {
        iconBg:     'bg-violet-500/15',
        iconText:   'text-violet-300',
        chipBg:     'bg-violet-500/15',
        chipText:   'text-violet-300',
      };
    case 'pink':
      return {
        iconBg:     'bg-pink-500/15',
        iconText:   'text-pink-300',
        chipBg:     'bg-pink-500/15',
        chipText:   'text-pink-300',
      };
    case 'amber':
      return {
        iconBg:     'bg-amber-500/15',
        iconText:   'text-amber-300',
        chipBg:     'bg-amber-500/15',
        chipText:   'text-amber-300',
      };
    case 'orange':
      return {
        iconBg:     'bg-orange-500/15',
        iconText:   'text-orange-300',
        chipBg:     'bg-orange-500/15',
        chipText:   'text-orange-300',
      };
    case 'red':
      return {
        iconBg:     'bg-red-500/15',
        iconText:   'text-red-300',
        chipBg:     'bg-red-500/15',
        chipText:   'text-red-300',
      };
    case 'teal':
      return {
        iconBg:     'bg-teal-500/15',
        iconText:   'text-teal-300',
        chipBg:     'bg-teal-500/15',
        chipText:   'text-teal-300',
      };
    case 'cyan':
      return {
        iconBg:     'bg-cyan-500/15',
        iconText:   'text-cyan-300',
        chipBg:     'bg-cyan-500/15',
        chipText:   'text-cyan-300',
      };
    default:
      return {
        iconBg:     'bg-blue-500/15',
        iconText:   'text-blue-300',
        chipBg:     'bg-blue-500/15',
        chipText:   'text-blue-300',
      };
  }
}

const HASH_WHEEL = ['emerald', 'blue', 'indigo', 'violet', 'pink', 'amber', 'orange', 'red', 'teal', 'cyan'];

function hashPick(seed: string): string {
  let h = 5381;
  for (let i = 0; i < seed.length; i++) {
    h = (h * 33) ^ seed.charCodeAt(i);
  }
  return HASH_WHEEL[Math.abs(h) % HASH_WHEEL.length];
}

function tierLabel(t: 'bundled' | 'user' | 'project'): string {
  switch (t) {
    case 'bundled': return 'Bundled';
    case 'user':    return 'User (~/.mira/skills)';
    case 'project': return 'Project (.mira/skills)';
  }
}

function tierChipClass(t: 'bundled' | 'user' | 'project'): string {
  switch (t) {
    case 'bundled': return 'bg-mira-blue/15 text-mira-blue border border-mira-blue/25';
    case 'user':    return 'bg-emerald-500/15 text-emerald-300 border border-emerald-500/25';
    case 'project': return 'bg-purple-500/15 text-purple-300 border border-purple-500/25';
  }
}

function tierDotClass(t: 'bundled' | 'user' | 'project'): string {
  switch (t) {
    case 'bundled': return 'bg-mira-blue';
    case 'user':    return 'bg-emerald-400';
    case 'project': return 'bg-purple-400';
  }
}

function timeAgoSecs(ts: number): string {
  const dt = Math.max(0, Math.floor((Date.now() - ts) / 1000));
  if (dt < 5) return 'just now';
  if (dt < 60) return `${dt}s ago`;
  if (dt < 3600) return `${Math.floor(dt / 60)}m ago`;
  return `${Math.floor(dt / 3600)}h ago`;
}

function ToggleField({
  label, hint, checked, onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className="flex items-start justify-between gap-3">
      <div className="min-w-0 flex-1">
        <div className="text-[12.5px] font-medium text-foreground/85">{label}</div>
        {hint && <div className="mt-1 text-[11.5px] text-muted-foreground/80">{hint}</div>}
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        onClick={() => onChange(!checked)}
        className={cn(
          'relative mt-1 inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors',
          checked ? 'bg-mira-blue' : 'bg-input',
        )}
      >
        <span
          className={cn(
            'inline-block size-4 rounded-full bg-white transition-transform',
            checked ? 'translate-x-4' : 'translate-x-0.5',
          )}
        />
      </button>
    </div>
  );
}

/**
 * Diff `next` against `current` and return a `MemoryUpdate` with only the
 * changed fields, or `undefined` if nothing changed. Sending only deltas
 * lets the backend distinguish "user cleared this" (present-with-null)
 * from "user didn't touch it" (absent).
 */
function memoryPatchFor(current: MemoryView, next: MemoryView): MemoryUpdate | undefined {
  const patch: MemoryUpdate = {};
  let any = false;
  if (current.auto_extract !== next.auto_extract) {
    patch.auto_extract = next.auto_extract;
    any = true;
  }
  if (current.tools_enabled !== next.tools_enabled) {
    patch.tools_enabled = next.tools_enabled;
    any = true;
  }
  if (current.inject_context !== next.inject_context) {
    patch.inject_context = next.inject_context;
    any = true;
  }
  const curModel = current.extractor_model ?? null;
  const nextModel = next.extractor_model ?? null;
  if (curModel !== nextModel) {
    patch.extractor_model = nextModel;
    any = true;
  }
  return any ? patch : undefined;
}

/* ---------- section: search & keys ---------- */

function KeysSection({
  view, draft, setDraft,
}: {
  view: SettingsView;
  draft: Draft;
  setDraft: (u: (d: Draft) => Draft) => void;
}) {
  function beginEdit(name: string) {
    setDraft((d) => ({
      ...d,
      keyEditing: { ...d.keyEditing, [name]: true },
      keyValues: { ...d.keyValues, [name]: '' },
    }));
  }
  function cancelEdit(name: string) {
    setDraft((d) => {
      const { [name]: _ignored1, ...restEditing } = d.keyEditing;
      const { [name]: _ignored2, ...restValues } = d.keyValues;
      return { ...d, keyEditing: restEditing, keyValues: restValues };
    });
  }
  function updateValue(name: string, value: string) {
    setDraft((d) => ({ ...d, keyValues: { ...d.keyValues, [name]: value } }));
  }

  return (
    <SectionShell
      title="Search & keys"
      subtitle="Third-party API keys tools consume. Stored in your mira.yaml and exported to the process env at startup so tools pick them up transparently."
    >
      <div className="flex flex-col gap-3">
        {view.keys.map((k) => {
          const meta = KEY_META[k.name];
          const editing = !!draft.keyEditing[k.name];
          const pending = draft.keyValues[k.name];
          const hasStored = k.masked.length > 0;
          return (
            <div
              key={k.name}
              className="rounded-lg border border-border/50 bg-background/40 p-3"
            >
              <div className="flex items-baseline justify-between gap-3">
                <div className="min-w-0 flex-1">
                  <div className="text-[13.5px] font-semibold text-foreground">
                    {meta?.label ?? k.name}
                  </div>
                  <div className="mt-0.5 font-mono text-[10.5px] text-muted-foreground/70">
                    {k.name}
                  </div>
                </div>
                {meta?.url && (
                  <a
                    href={meta.url}
                    target="_blank"
                    rel="noreferrer"
                    className="text-[11.5px] text-mira-blue hover:underline"
                  >
                    Get key →
                  </a>
                )}
              </div>
              {meta && (
                <div className="mt-1 text-[12px] text-muted-foreground">{meta.help}</div>
              )}

              <div className="mt-2.5">
                {!editing && hasStored && (
                  <SavedBadge
                    tone={k.from_env ? 'blue' : 'green'}
                    icon={k.from_env ? <Key className="size-3.5" /> : <Check className="size-3.5" />}
                    label={k.from_env ? 'From env (yaml overrides)' : 'Saved'}
                    masked={k.masked}
                    onClick={() => beginEdit(k.name)}
                    action="Replace"
                  />
                )}
                {!editing && !hasStored && k.from_env && (
                  <SavedBadge
                    tone="blue"
                    icon={<Key className="size-3.5" />}
                    label="Using env value"
                    masked={`$${k.name}`}
                    onClick={() => beginEdit(k.name)}
                    action="Override"
                  />
                )}
                {!editing && !hasStored && !k.from_env && (
                  <Button variant="outline" size="sm" onClick={() => beginEdit(k.name)}>
                    Add key
                  </Button>
                )}
                {editing && (
                  <div className="flex gap-2">
                    <Input
                      type="password"
                      value={pending ?? ''}
                      onChange={(e) => updateValue(k.name, e.target.value)}
                      placeholder="Paste key…"
                      autoComplete="off"
                      autoFocus
                      spellCheck={false}
                    />
                    <Button variant="outline" size="sm" onClick={() => cancelEdit(k.name)}>
                      Cancel
                    </Button>
                  </div>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </SectionShell>
  );
}

/* ---------- section: about ---------- */

function AboutSection({ view }: { view: SettingsView }) {
  return (
    <SectionShell title="About" subtitle="Where Mira reads and writes settings on this machine.">
      <Field label="Config file">
        <div className="flex min-w-0 items-center gap-2 rounded-md border border-border/50 bg-background/40 px-2.5 py-1.5 text-[12.5px]">
          <FolderOpen className="size-3.5 shrink-0 text-muted-foreground" />
          <code className="min-w-0 truncate font-mono text-foreground/85">{view.config_path}</code>
        </div>
      </Field>
      <Field label="Status">
        <div
          className={cn(
            'inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[12.5px]',
            view.configured
              ? 'bg-emerald-500/10 text-emerald-400'
              : 'bg-amber-500/10 text-amber-300',
          )}
        >
          {view.configured ? <Check className="size-3.5" /> : <ArrowClockwise className="size-3.5" />}
          {view.configured ? 'Configured — provider ready' : 'Needs API key'}
        </div>
      </Field>
    </SectionShell>
  );
}

/* ---------- shared bits ---------- */

function SectionShell({
  title, subtitle, children,
}: {
  title: string;
  subtitle?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-4">
      <div>
        <div className="text-[16px] font-semibold text-foreground">{title}</div>
        {subtitle && <div className="mt-1 text-[12.5px] text-muted-foreground">{subtitle}</div>}
      </div>
      <div className="flex flex-col gap-4">{children}</div>
    </div>
  );
}

function Field({
  label, hint, children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <label className="text-[12.5px] font-medium text-foreground/85">{label}</label>
      {children}
      {hint && <div className="text-[11.5px] text-muted-foreground/80">{hint}</div>}
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
    <Field label="API key" hint="Stored in your mira.yaml. Never sent to the model.">
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
    </Field>
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
          ? 'border-emerald-500/40 bg-emerald-500/[0.06]'
          : 'border-mira-blue/40 bg-mira-blue/[0.06]',
      )}
    >
      <span className={tone === 'green' ? 'text-emerald-400' : 'text-mira-blue'}>{icon}</span>
      <span className="text-muted-foreground">{label}</span>
      <span className="flex-1 min-w-0 font-mono text-xs overflow-hidden text-ellipsis whitespace-nowrap">{masked}</span>
      <Button variant="outline" size="sm" onClick={onClick} type="button">{action}</Button>
    </div>
  );
}
