import React, { useEffect, useMemo, useState } from 'react';
import { OAUTH_PROVIDERS, PROVIDER_FAVICON_DOMAIN, PROVIDER_PRESETS } from '../lib/providers';
import { needsLightTile } from '../lib/models';
import {
  Bot,
  Smile,
  RotateCw,
  Book,
  BookOpen,
  Braces,
  Brain,
  Bug,
  Bell,
  Check,
  ChevronRight,
  Code,
  Database,
  Eye,
  FileText,
  FolderOpen,
  Cog,
  GitBranch,
  GitCompare,
  GitCommit,
  GitMerge,
  GitPullRequest,
  Info,
  Key,
  Keyboard,
  Lightbulb,
  MessagesSquare,
  Zap,
  Search,
  NotebookPen,
  Package,
  Palette,
  Paperclip,
  PenLine,
  Plug,
  Rocket,
  ScanFace,
  Shield,
  ShieldCheck,
  PanelLeft,
  Sparkle,
  SquareTerminal,
  Target,
  Terminal,
  Wrench,
  Globe2,
  Timer,
} from 'lucide-react';
import {
  getSettings,
  getSkill,
  listSkills,
  putSettings,
  reloadSkills,
  type SkillDetail,
  type SkillView,
} from '../api';
export type { GithubReturn } from './Integrations';
import { Markdown } from './Markdown';
import { IntegrationsSection, type GithubReturn } from './Integrations';
import { UsageSection } from './UsageSection';
import { ChartColumn } from 'lucide-react';
import { HooksSection } from './Hooks';
import { KeybindingsSection } from './settings/KeybindingsSettings';
import { Collapse } from './ui/Collapse';
import { AcpAgentsSection } from './settings/AcpAgentsSection';
import { SubagentsSection } from './settings/SubagentsSection';
import { ImportChats } from './ImportChats';
import type { AcpAgentStatus, SessionsSettings } from '../types';
import { applyReduceMotion, PREF_KEYS, useBoolPref, useStringPref } from '@/lib/prefs';
import { useTheme, useThemePref, type ThemePref } from '@/lib/theme';
import { ACCENT_SWATCH, ACCENTS, setAccent, setUiScale, useAccent, useUiScale, type UiScale } from '@/lib/appearance';
import { playTurnSound } from '@/lib/sound';
import { useDiffOptions } from '@/lib/diffPrefs';
import { parseDiffFromFile } from '@pierre/diffs';
import { StyledDiffCodeView } from './diffs/StyledDiffCodeView';
import { getPreferredEditorId, listEditors } from '@/lib/editors';
import { EditorIcon } from './EditorPicker';
import { isMacPlatform } from '@/lib/keybindings';
import type {
  KeyUpdate,
  MemoryUpdate,
  MemoryView,
  Mode,
  ProviderUpdate,
  SettingsView,
} from '../types';
import { Dialog, DialogContent } from '@/components/ui/dialog';
import { SectionInput } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import openrouterIcon from '../assets/openrouter-icon.png';
import chatgptIcon from '../assets/chatgpt-icon.svg';

/**
 * Two-pane settings: sidebar of sections on the left, the active section's
 * fields on the right. State is pooled at this level so every section
 * edits the same `draft` and a single "Save" commits everything at once —
 * users can jump between sections without losing work.
 */



/** Brand mark for a provider preset: bundled art for the OAuth providers,
 *  live site favicon otherwise (hidden if it fails to load, e.g. offline). */
function ProviderIcon({ name }: { name: string }) {
  const [failed, setFailed] = useState(false);
  const bundled =
    name === 'openrouter' ? openrouterIcon : name === 'openai' ? chatgptIcon : null;
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
      className={needsLightTile(name) ? 'size-3 object-contain' : 'size-4 shrink-0 rounded-[4px] object-contain'}
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
    help: 'Powers the pull-request panel and connecting repositories (Integrations). The link opens a new-token page with the `repo` and `workflow` scopes pre-selected.',
    url: 'https://github.com/settings/tokens/new?scopes=repo,workflow&description=Mira',
  },
  SLACK_BOT_TOKEN: {
    label: 'Slack bot token',
    help: 'The xoxb-… token from your Slack app’s OAuth & Permissions page. Used by `mira slack`.',
    url: 'https://api.slack.com/apps',
  },
  SLACK_APP_TOKEN: {
    label: 'Slack app token',
    help: 'An app-level xapp-… token with the connections:write scope (Basic Information → App-Level Tokens). Used by `mira slack`.',
    url: 'https://api.slack.com/apps',
  },
};

export type SettingsSectionId = 'general' | 'appearance' | 'provider' | 'agents' | 'subagents' | 'usage' | 'memory' | 'skills' | 'hooks' | 'keybindings' | 'search' | 'integrations' | 'about';

/** Section metadata exported so the Sidebar can render the same nav in
 *  its "settings mode" (the settings surface is now inline in the main
 *  pane, not a dialog — the sidebar drives section selection). */
export const SETTINGS_SECTIONS: {
  id: SettingsSectionId;
  label: string;
  icon: React.ComponentType<{ className?: string }>;
}[] = [
  { id: 'general',     label: 'General',     icon: Cog },
  { id: 'appearance',  label: 'Appearance',  icon: Palette },
  { id: 'provider',    label: 'Provider',    icon: Plug },
  // External coding agents. A sibling of Provider, not a child: they
  // authenticate and bill separately, so a Provider key does not apply.
  { id: 'agents',      label: 'External agents', icon: Bot },
  // Mira's own helpers — a different thing from external agents, hence
  // a different word everywhere.
  { id: 'subagents',   label: 'Subagents',   icon: Smile },
  { id: 'usage',       label: 'Usage',       icon: ChartColumn },
  { id: 'memory',      label: 'Memory',      icon: Brain },
  { id: 'skills',      label: 'Skills',      icon: Sparkle },
  { id: 'hooks',       label: 'Hooks',       icon: Zap },
  { id: 'keybindings', label: 'Keyboard shortcuts', icon: Keyboard },
  { id: 'search',      label: 'Search & keys', icon: Search },
  { id: 'integrations', label: 'Integrations', icon: Plug },
  { id: 'about',       label: 'About',       icon: Info },
];

// Local alias — the exported name is `SETTINGS_SECTIONS` (used by the
// Sidebar); everywhere inside this file we still refer to it as
// `SECTIONS` to keep the diff tight.
const SECTIONS = SETTINGS_SECTIONS;

type SurfaceProps = {
  /** Which section to render — driven by the sidebar. */
  section: SettingsSectionId;
  onSectionChange: (id: SettingsSectionId) => void;
  onSaved: (v: SettingsView) => void;
  /** Called when the user hits "Back to App" or otherwise leaves
   *  settings — the caller pops back to the previous main view. */
  onExit: () => void;
  /** Bumps whenever the backend broadcasts `skills_reloaded` (fs
   *  watcher detected a `SKILL.md` change). Threaded into the Skills
   *  panel so its list re-fetches automatically without the user
   *  clicking Reload. */
  skillsVersion?: number;
  /** Set after GitHub sends the user back from installing the app. */
  githubReturn?: GithubReturn;

  // -------- external ACP agents --------
  //
  // Kept out of `SettingsView` deliberately: a provider is server-side
  // configuration, while an agent's binary path and launch args describe how
  // to run a third-party binary on *this* machine. Persisted client-side.
  acpAgents?: AcpAgentStatus[];
  acpRefreshing?: boolean;
  acpDriver?: string | null;
  acpError?: string | null;
  onAcpRefresh?: () => void;
  onAcpStart?: (kind: string, resume?: string | null) => void;
};

type Draft = {
  providerName: string;
  baseUrl: string;
  apiKey: string;
  showReplaceKey: boolean;
  model: string;
  smallModel: string;
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
  smallModel: '',
  mode: 'manual',
  maxTokens: '',
  keyValues: {},
  keyEditing: {},
  memory: EMPTY_MEMORY,
};

export function SettingsSurface({
  section,
  onSectionChange,
  onSaved,
  onExit,
  skillsVersion = 0,
  githubReturn = null,
  acpAgents = [],
  acpRefreshing = false,
  acpDriver = null,
  acpError = null,
  onAcpRefresh = () => {},
  onAcpStart = () => {},
}: SurfaceProps) {
  const [view, setView] = useState<SettingsView | null>(null);
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
      .catch(() => { /* leave stale view; user can retry */ });
  };

  // Load once on mount. The surface persists across section swaps
  // (the sidebar changes `section` without unmounting us), so we
  // shouldn't re-hydrate on every section click — that would drop
  // in-progress edits when the user navigates away and back.
  useEffect(() => {
    setLoadError(null);
    setSaveError(null);
    getSettings()
      .then((v) => { setView(v); hydrate(v); })
      .catch((e) => setLoadError(String(e.message ?? e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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
        <span className="min-w-0 flex-1 truncate text-[13.5px] text-foreground">{currentLabel}</span>
        {view && (
          <div
            className={cn(
              'inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[10.5px]',
              view.configured
                ? 'bg-emerald-500/10 text-emerald-400'
                : 'bg-amber-500/10 text-amber-300',
            )}
          >
            <span className={cn('size-1.5 rounded-full', view.configured ? 'bg-emerald-500' : 'bg-amber-500')} />
            {view.configured ? 'Configured' : 'Needs API key'}
          </div>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        <div
          key={section}
          className={cn(
            'mx-auto w-full animate-fade-in px-6 py-6',
          section === 'usage' ? 'max-w-3xl' :
          section === 'hooks' || section === 'keybindings' || section === 'skills' || section === 'agents' || section === 'subagents' ? 'max-w-5xl' :
          'max-w-2xl',
        )}>
          {loadError && (
            <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-[12.5px] text-destructive">
              error: {loadError}
            </div>
          )}
          {!view && !loadError && (
            <div className="text-[12.5px] text-muted-foreground">loading…</div>
          )}

          {view && section === 'provider' && (
            <>
              <div className="mb-4 rounded-lg border border-border/70 bg-muted/30 px-3 py-2.5 text-[11.5px] leading-relaxed text-muted-foreground">
                These credentials are for models{' '}
                <span className="text-foreground/85">Mira</span> calls
                directly. External coding agents — Claude Code, Codex,
                Cursor and the rest — bring their own login and are configured
                under{' '}
                <button
                  type="button"
                  onClick={() => onSectionChange('agents')}
                  className="text-mira-blue/85 underline underline-offset-2 hover:text-mira-blue"
                >
                  External agents
                </button>
                .
              </div>
              <ProviderSection view={view} draft={draft} setDraft={setDraft} refetch={refetch} />
            </>
          )}
          {section === 'agents' && (
            <>
              <AcpAgentsSection
                agents={acpAgents}
                refreshing={acpRefreshing}
                onRefresh={onAcpRefresh}
                onStart={onAcpStart}
                activeKind={acpDriver}
                error={acpError}
              />
              <ImportChatsCard />
            </>
          )}
          {section === 'subagents' && <SubagentsSection />}
          {view && section === 'general' && (
            <GeneralSection draft={draft} setDraft={setDraft} />
          )}
          {section === 'appearance' && <AppearanceSection />}
          {view && section === 'memory' && (
            <MemorySection draft={draft} setDraft={setDraft} />
          )}
          {view && section === 'skills' && (
            <SkillsSection version={skillsVersion} />
          )}
          {view && section === 'search' && (
            <KeysSection view={view} draft={draft} setDraft={setDraft} />
          )}
          {view && section === 'hooks' && <HooksSection />}
          {section === 'keybindings' && <KeybindingsSection />}
          {section === 'usage' && <UsageSection />}
          {view && section === 'integrations' && (
            <IntegrationsSection onOpenKeys={() => onSectionChange('search')} githubReturn={githubReturn} />
          )}
          {view && section === 'about' && (
            <AboutSection view={view} />
          )}
        </div>
      </div>

      {view && (
        <div className="flex shrink-0 items-center justify-between border-t border-border/60 px-6 py-3">
          <div className="text-[11.5px] text-muted-foreground">
            {saveError ? <span className="text-destructive">{saveError}</span>
              : dirty ? 'Unsaved changes'
              : 'Up to date'}
          </div>
          <div className="flex items-center gap-2">
            <Button variant="outline" onClick={onExit} disabled={saving}>Back to app</Button>
            <Button onClick={onSave} disabled={saving || !dirty}>
              {saving ? 'Saving…' : 'Save'}
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}

/* ---------- section: provider ---------- */

function ProviderSection({
  view, draft, setDraft, refetch,
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

/* ---------- section: general ---------- */

function GeneralSection({
  draft, setDraft,
}: {
  draft: Draft;
  setDraft: (u: (d: Draft) => Draft) => void;
}) {
  const [sidebarOpen, setSidebarOpen] = useBoolPref(PREF_KEYS.sidebarOpen, true);
  const [terminalRestore, setTerminalRestore] = useBoolPref(PREF_KEYS.terminalRestore, true);
  const [cmdEnterSend, setCmdEnterSend] = useBoolPref(PREF_KEYS.composerCmdEnter, false);
  const [follow, setFollow] = useBoolPref(PREF_KEYS.transcriptFollow, true);
  const [turnStats, setTurnStats] = useBoolPref(PREF_KEYS.transcriptTurnStats, true);
  const [notifyTurnDone, setNotifyTurnDone] = useBoolPref(PREF_KEYS.notifyTurnDone, false);
  const [turnSound, setTurnSound] = useBoolPref(PREF_KEYS.turnSound, true);
  const [browserAutoOpen, setBrowserAutoOpen] = useBoolPref(PREF_KEYS.browserAutoOpen, true);
  // Server-side: these act with no window open, so they live in Mira's
  // config rather than this browser's storage.
  const [sessionsCfg, setSessionsCfg] = useState<SessionsSettings | null>(null);
  const [sessionsError, setSessionsError] = useState<string | null>(null);
  useEffect(() => {
    getSettings()
      .then((v) => setSessionsCfg(v.sessions ?? { auto_resume_after_limit: false, keep_awake_while_running: false }))
      .catch((e) => setSessionsError(String((e as Error).message ?? e)));
  }, []);
  function setSessionsFlag(key: keyof SessionsSettings, value: boolean) {
    const prev = sessionsCfg;
    setSessionsCfg((c) => (c ? { ...c, [key]: value } : c));
    setSessionsError(null);
    putSettings({ sessions: { [key]: value } })
      .then((v) => v.sessions && setSessionsCfg(v.sessions))
      .catch((e) => {
        setSessionsCfg(prev);
        setSessionsError(String((e as Error).message ?? e));
      });
  }
  const [preferredEditor, setPreferredEditor] = useStringPref(PREF_KEYS.preferredEditor, '');
  const [editorOptions, setEditorOptions] = useState<{ value: string; label: string; icon?: React.ReactNode }[]>([]);
  const [notifyState, setNotifyState] = useState(() =>
    typeof Notification === 'undefined' ? 'unsupported' : Notification.permission,
  );

  useEffect(() => {
    let cancelled = false;
    listEditors()
      .then((v) => {
        if (cancelled) return;
        const detected = new Set(v.editors.map((e) => e.id));
        setEditorOptions(
          v.all.map((e) => ({
            value: e.id,
            label: detected.has(e.id) ? e.name : `${e.name} (not detected)`,
            icon: <EditorIcon entry={e} />,
          })),
        );
        if (!getPreferredEditorId() && v.default_id) setPreferredEditor(v.default_id);
      })
      .catch(() => {
        /* offline / server down — row keeps its placeholder */
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function setNotify(v: boolean) {
    if (v && typeof Notification !== 'undefined' && Notification.permission === 'default') {
      try {
        setNotifyState(await Notification.requestPermission());
      } catch {
        setNotifyState(Notification.permission);
      }
    }
    setNotifyTurnDone(v);
    if (typeof Notification !== 'undefined') setNotifyState(Notification.permission);
  }

  return (
    <div className="flex flex-col gap-2.5">
      <TSection
        icon={<Cog className="size-3.5" />}
        title="General"
        description="Defaults new sessions inherit. Override per-session via the composer chips."
      >
        <TRow
          title="Default mode"
          description={MODE_DESCRIPTIONS[draft.mode]}
          control={
            <Select<Mode>
              value={draft.mode}
              onChange={(v) => setDraft((d) => ({ ...d, mode: v }))}
              options={MODES.map((m) => ({
                value: m,
                label: m,
                hint: MODE_DESCRIPTIONS[m],
              }))}
              className="h-8 w-full text-[13px] sm:w-64"
            />
          }
        />

        <TRow
          title="Max tokens"
          description="Cap on tokens the model can emit per response. Leave blank for the provider default."
          control={
            <SectionInput
              type="number"
              value={draft.maxTokens}
              onChange={(e) => setDraft((d) => ({ ...d, maxTokens: e.target.value }))}
              min={1}
              placeholder="(provider default)"
              className="h-8 text-[13px] sm:w-64"
            />
          }
        />
      </TSection>

      <TSection
        icon={<PenLine className="size-3.5" />}
        title="Composer"
        description="How the message box sends. Stored in this browser only."
      >
        <TRow
          title="Send with ⌘/Ctrl+Enter"
          description={isMacPlatform() ? 'Plain Enter inserts a newline; ⌘+Enter sends. Off: Enter sends, Shift+Enter is the newline.' : 'Plain Enter inserts a newline; Ctrl+Enter sends. Off: Enter sends, Shift+Enter is the newline.'}
          control={
            <TSwitch checked={cmdEnterSend} onChange={setCmdEnterSend} label="Send with mod+Enter" />
          }
        />
      </TSection>

      <TSection
        icon={<MessagesSquare className="size-3.5" />}
        title="Transcript"
        description="How the conversation reads. Stored in this browser only."
      >
        <TRow
          title="Auto-scroll while streaming"
          description="Stick to the latest output as it arrives. Scroll up to pause and get a jump-to-latest button."
          control={
            <TSwitch checked={follow} onChange={setFollow} label="Auto-scroll while streaming" />
          }
        />

        <TRow
          title="Per-turn timing & cost"
          description="Show “worked for Xs · N in · M out · $Y” chips on finished turns and hover actions."
          control={
            <TSwitch checked={turnStats} onChange={setTurnStats} label="Per-turn timing and cost" />
          }
        />
      </TSection>

      <TSection
        icon={<Bell className="size-3.5" />}
        title="Notifications"
        description="System alerts from this browser. Stored in this browser only."
      >
        <TRow
          title="Notify when Mira finishes or needs you"
          description={
            notifyState === 'denied'
              ? 'Blocked — allow notifications for this site in your browser settings, then turn this back on.'
              : notifyState === 'unsupported'
                ? 'This browser does not support desktop notifications.'
                : 'A system alert when a chat finishes, asks for approval, or has a question — only while Mira is in the background.'
          }
          status={
            notifyTurnDone && notifyState === 'granted'
              ? 'On — you’ll hear from Mira while it’s in the background.'
              : undefined
          }
          control={
            <TSwitch
              checked={notifyTurnDone && notifyState !== 'denied' && notifyState !== 'unsupported'}
              onChange={(v) => void setNotify(v)}
              label="Notify when Mira finishes or needs you"
            />
          }
        />
        <TRow
          title="Sound when a reply finishes"
          description="A short chime when Mira or an agent finishes a turn."
          control={
            <>
              <button
                type="button"
                onClick={() => playTurnSound(true)}
                className="h-7 rounded-md px-2.5 text-[12px] text-muted-foreground transition-colors hover:bg-fg/[0.06] hover:text-foreground"
              >
                Play
              </button>
              <TSwitch checked={turnSound} onChange={setTurnSound} label="Sound when a reply finishes" />
            </>
          }
        />
      </TSection>

      <TSection
        icon={<Timer className="size-3.5" />}
        title="Running sessions"
        description="What chats do while they run on their own. Saved in Mira's config, so it applies even with this window closed."
      >
        <TRow
          title="Auto resume after a limit resets"
          description="When a chat stops on a usage limit, pick it up again automatically once the limit resets, instead of waiting for you to press Resume."
          control={
            <TSwitch
              checked={sessionsCfg?.auto_resume_after_limit ?? false}
              onChange={(v) => setSessionsFlag('auto_resume_after_limit', v)}
              label="Auto resume after a limit resets"
            />
          }
        />
        <TRow
          title="Keep the screen awake while a session runs"
          description="Stops the display and the computer from sleeping while any chat is working. Released as soon as nothing is running. macOS and Linux."
          control={
            <TSwitch
              checked={sessionsCfg?.keep_awake_while_running ?? false}
              onChange={(v) => setSessionsFlag('keep_awake_while_running', v)}
              label="Keep the screen awake while a session runs"
            />
          }
        />
        {sessionsError && <p className="px-1 text-[12px] text-destructive">{sessionsError}</p>}
      </TSection>

      <TSection
        icon={<Globe2 className="size-3.5" />}
        title="Browser"
        description="The browser Mira and its agents share. Stored in this browser only."
      >
        <TRow
          title="Show the browser when it's used"
          description="Open the browser pane as soon as Mira or an agent starts browsing, so you can watch and take over."
          control={
            <TSwitch
              checked={browserAutoOpen}
              onChange={setBrowserAutoOpen}
              label="Show the browser when it's used"
            />
          }
        />
      </TSection>

      <TSection
        icon={<SquareTerminal className="size-3.5" />}
        title="External editor"
        description="Where files and folders open. The toolbar picker and file viewer use this. Stored in this browser only."
      >
        <TRow
          title="Preferred editor"
          description="Detected editors are listed first. Anything else is tried anyway — the server reports if it isn't installed."
          control={
            <Select<string>
              value={preferredEditor}
              onChange={(v) => setPreferredEditor(v)}
              options={editorOptions}
              placeholder={editorOptions.length === 0 ? 'Loading…' : 'System default'}
              className="h-8 w-full text-[13px] sm:w-64"
            />
          }
        />
      </TSection>

      <TSection
        icon={<PanelLeft className="size-3.5" />}
        title="Startup"
        description="How the app shell looks when you open Mira. Stored in this browser only."
      >
        <TRow
          title="Show sidebar on startup"
          description="Keep the session list visible. Turn off for a full-width transcript."
          control={
            <TSwitch checked={sidebarOpen} onChange={setSidebarOpen} label="Show sidebar on startup" />
          }
        />

        <TRow
          title="Restore terminal on startup"
          description="Reopen the integrated terminal if it was open last time. Turn off to always start with it closed."
          control={
            <TSwitch checked={terminalRestore} onChange={setTerminalRestore} label="Restore terminal on startup" />
          }
        />
      </TSection>
    </div>
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
    <div className="flex flex-col gap-2.5">
      <TSection
        icon={<Brain className="size-3.5" />}
        title="Memory"
        description="Cross-session memory: user + project MIRA.md, plus the agent-written episodic stream at .mira/episodic.jsonl. Changes apply to new chats — click New chat after saving to try them."
      >
        <TRow
          title="Inject memory into prompt"
          description="Add a live 'memory' section to every model request. Turn off to shrink the system prompt back to the pre-memory baseline — useful for isolating whether the injected content is confusing the model."
          control={
            <TSwitch checked={draft.memory.inject_context} onChange={(v) => update('inject_context', v)} label="Inject memory into prompt" />
          }
        />

        <TRow
          title="Enable memory tools"
          description="Registers memory_read / memory_search / memory_append / memory_edit / memory_remember. Turn off to remove them from the model's tool list — useful when the extra tools distract simple questions."
          control={
            <TSwitch checked={draft.memory.tools_enabled} onChange={(v) => update('tools_enabled', v)} label="Enable memory tools" />
          }
        />

        <TRow
          title="Auto-extract facts after each turn"
          description="Background pass that mines each finished turn for durable facts and appends them to .mira/episodic.jsonl. Only fires when at least one tool call succeeded."
          control={
            <TSwitch checked={draft.memory.auto_extract} onChange={(v) => update('auto_extract', v)} label="Auto-extract facts after each turn" />
          }
        />

        <TRow
          title="Extractor model"
          description="Model id used for the extraction call. Leave blank to reuse the session's active model (works but is expensive). Point at your provider's cheap tier — e.g. claude-haiku-4-5, gpt-5-nano, deepseek-chat — for negligible per-round cost."
          control={
            <SectionInput
              value={draft.memory.extractor_model ?? ''}
              onChange={(e) => update('extractor_model', e.target.value || null)}
              placeholder="(uses session model)"
              spellCheck={false}
              className="h-8 text-[13px] sm:w-64"
            />
          }
        />
      </TSection>

      <TNote>
        <span>
          Settings are saved to <code className="font-mono">mira.yaml</code>. To apply them to the
          running server, <b>restart</b> Mira (Ctrl+C then run the command again). Hot-reload
          without restart is not yet wired.
        </span>
      </TNote>
    </div>
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
function SkillsSection({ version = 0 }: { version?: number }) {
  const [skills, setSkills] = useState<SkillView[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastReload, setLastReload] = useState<number | null>(null);
  // Name of the skill whose detail drawer is open (null = closed). The
  // drawer fetches the full payload lazily on open so the list-view
  // fetch stays cheap.
  const [selected, setSelected] = useState<string | null>(null);

  useEffect(() => {
    setLoading(true);
    listSkills()
      .then((s) => {
        setSkills(s);
        setError(null);
        // Stamp reload time on any fetch trigger so the "reloaded Xs
        // ago" hint applies to auto-reloads (via the fs watcher) too.
        if (version > 0) setLastReload(Date.now());
      })
      .catch((e) => setError(String((e as Error).message)))
      .finally(() => setLoading(false));
  }, [version]);

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
    const out: Record<'bundled' | 'shared' | 'user' | 'project', SkillView[]> = {
      bundled: [],
      shared: [],
      user: [],
      project: [],
    };
    for (const s of skills ?? []) {
      out[s.tier].push(s);
    }
    (['bundled', 'shared', 'user', 'project'] as const).forEach((k) => {
      out[k].sort((a, b) => a.name.localeCompare(b.name));
    });
    return out;
  }, [skills]);

  const total = skills?.length ?? 0;

  return (
    <div className="flex flex-col gap-2.5">
      <TSection
        icon={<Sparkle className="size-3.5" />}
        title="Skills"
        description="Reusable instruction bundles the agent invokes to accomplish a specific task. Add your own to ~/.mira/skills/ (user-wide) or <cwd>/.mira/skills/ (per-repo)."
        action={
          <>
            <span className="text-[11px] tabular-nums text-muted-foreground">
              {loading ? 'Loading…' : `${total} skill${total === 1 ? '' : 's'}`}
              {lastReload && !loading ? ` · reloaded ${timeAgoSecs(lastReload)}` : ''}
            </span>
            <button
              type="button"
              onClick={reload}
              disabled={loading}
              title="Re-read skill files from disk"
              aria-label="Reload skills from disk"
              className="rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40"
            >
              <RotateCw className={cn('size-3.5', loading && 'animate-spin')} strokeWidth={2.5} />
            </button>
          </>
        }
      >
        {error && (
          <div className="border-b border-border/50 px-4 py-2.5 text-[12px] text-destructive">
            {error}
          </div>
        )}

        {!error && !loading && skills != null && total === 0 && (
          <EmptySkillsState />
        )}

        {(['bundled', 'shared', 'user', 'project'] as const).map((tier) => {
          const entries = grouped[tier];
          if (entries.length === 0) return null;
          return (
            <div key={tier}>
              <div className="flex items-center gap-2 border-b border-border/50 px-4 pb-2 pt-3">
                <span className={cn('size-1.5 rounded-full', tierDotClass(tier))} />
                <span className="text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
                  {tierLabel(tier)}
                </span>
                <span className="text-[11px] tabular-nums text-muted-foreground/70">{entries.length}</span>
              </div>
              <div className="[&>*+*]:border-t [&>*+*]:border-border/50">
                {entries.map((s) => (
                  <SkillRow
                    key={`${tier}-${s.name}`}
                    skill={s}
                    onOpen={() => setSelected(s.name)}
                  />
                ))}
              </div>
            </div>
          );
        })}
      </TSection>

      <SkillDetailDialog
        name={selected}
        onClose={() => setSelected(null)}
      />
    </div>
  );
}

/** One skill as a row: icon square, name + description, chevron.
 *  Colour lives only on the icon square — the visual anchor that lets the
 *  eye pick a skill out without overwhelming the list. */
function SkillRow({ skill, onOpen }: { skill: SkillView; onOpen: () => void }) {
  const iconKey = skill.icon ?? defaultIconKey(skill);
  const Icon = iconFor(iconKey);
  const palette = paletteFor(skill.color, skill.name);

  return (
    <button
      type="button"
      onClick={onOpen}
      className="group flex w-full items-center gap-3 px-4 py-2.5 text-left transition-colors hover:bg-accent/40 focus:outline-none"
    >
      <div
        className={cn(
          'inline-flex size-8 shrink-0 items-center justify-center rounded-lg',
          palette.iconBg,
          palette.iconText,
        )}
      >
        <Icon className="size-4" />
      </div>

      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="truncate font-mono text-[13px] font-semibold text-foreground">
            /{skill.name}
          </span>
          {skill.category && (
            <span className={cn(
              'shrink-0 rounded-sm px-1.5 py-px text-[10px] font-medium uppercase tracking-wider',
              palette.chipBg,
              palette.chipText,
            )}>
              {skill.category}
            </span>
          )}
          {skill.has_attachments && (
            <Paperclip className="size-3 shrink-0 text-muted-foreground/60" />
          )}
        </div>
        <p className="mt-px truncate text-[12px] text-muted-foreground/85">
          {skill.description}
        </p>
      </div>

      <ChevronRight className="size-3.5 shrink-0 text-muted-foreground/50 transition-all group-hover:translate-x-px group-hover:text-foreground" />
    </button>
  );
}

/** Detail drawer for one skill — opens on card click, fetches the full
 *  SKILL.md body + attached files from `/api/skills/:name`, and renders
 *  the body as markdown. `name === null` closes the dialog; a name-swap
 *  transitions cleanly by keying the loader on `name`. */
function SkillDetailDialog({
  name,
  onClose,
}: {
  name: string | null;
  onClose: () => void;
}) {
  const [detail, setDetail] = useState<SkillDetail | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (name === null) {
      setDetail(null);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    getSkill(name)
      .then((d) => {
        if (cancelled) return;
        if (d) setDetail(d);
        else setError(`no skill named ${name}`);
      })
      .catch((e) => {
        if (!cancelled) setError(String((e as Error).message));
      })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [name]);

  const open = name !== null;

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-2xl p-0 gap-0 overflow-hidden">
        <div className="flex h-[560px] max-h-[80vh] flex-col">
          {/* Header: icon badge + name + tier chip. Mirrors the card
              styling so it feels like the card expanded rather than a
              separate view. */}
          {detail ? (
            <SkillDetailHeader detail={detail} />
          ) : (
            <div className="flex items-center gap-3 border-b border-border/60 px-5 py-4">
              <div className="size-9 shrink-0 rounded-lg bg-secondary/60" />
              <div className="flex min-w-0 flex-1 flex-col gap-1">
                <div className="h-4 w-32 rounded bg-secondary/60" />
                <div className="h-3 w-52 rounded bg-secondary/40" />
              </div>
            </div>
          )}

          <div className="flex-1 overflow-y-auto px-5 py-4">
            {loading && !detail && (
              <div className="text-[12.5px] text-muted-foreground">loading…</div>
            )}
            {error && (
              <div className="rounded-md border border-destructive/40 bg-destructive/[0.08] px-3 py-2 text-[12px] text-destructive">
                {error}
              </div>
            )}
            {detail && (
              <div className="flex flex-col gap-4">
                {/* Body — the SKILL.md markdown. The invoked skill sees
                    this exact text as a system-reminder, so showing it
                    verbatim doubles as documentation for what the model
                    is about to do. */}
                <div className="rounded-lg border border-border/50 bg-background/40 px-4 py-3">
                  {detail.body.trim().length === 0 ? (
                    <div className="text-[12px] italic text-muted-foreground">
                      (this skill has no body)
                    </div>
                  ) : (
                    <Markdown text={detail.body} />
                  )}
                </div>

                {detail.attachments.length > 0 && (
                  <div className="flex flex-col gap-1.5">
                    <div className="text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
                      Attached files ({detail.attachments.length})
                    </div>
                    <ul className="flex flex-col gap-1 rounded-md border border-border/50 bg-secondary/20 px-3 py-2">
                      {detail.attachments.map((f) => (
                        <li
                          key={f}
                          className="flex items-center gap-2 font-mono text-[11.5px] text-foreground/85"
                        >
                          <Paperclip className="size-3 shrink-0 text-muted-foreground" />
                          {f}
                        </li>
                      ))}
                    </ul>
                  </div>
                )}

                {detail.source && (
                  <div className="flex flex-col gap-1.5">
                    <div className="text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
                      Source
                    </div>
                    <div className="rounded-md border border-border/50 bg-secondary/20 px-3 py-2 font-mono text-[11.5px] text-foreground/80 break-all">
                      {detail.source}
                    </div>
                  </div>
                )}

                {!detail.source && (
                  <div className="text-[11.5px] text-muted-foreground">
                    Bundled skill — ships with the binary. Drop a file at{' '}
                    <code className="font-mono text-foreground/85">
                      ~/.mira/skills/{detail.name}/SKILL.md
                    </code>{' '}
                    to override.
                  </div>
                )}
              </div>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function SkillDetailHeader({ detail }: { detail: SkillDetail }) {
  const iconKey = detail.icon ?? defaultIconKey(detail);
  const Icon = iconFor(iconKey);
  const palette = paletteFor(detail.color, detail.name);
  return (
    <div className="flex items-start gap-3 border-b border-border/60 px-5 py-4">
      <div
        className={cn(
          'inline-flex size-9 shrink-0 items-center justify-center rounded-lg',
          palette.iconBg,
          palette.iconText,
        )}
      >
        <Icon className="size-5" />
      </div>
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex items-center gap-2">
          <span className="truncate font-mono text-[14px] font-semibold text-foreground">
            /{detail.name}
          </span>
          <span className="shrink-0 rounded-full border border-border/60 bg-secondary px-2 py-0.5 text-[9.5px] font-semibold uppercase tracking-wider text-muted-foreground">
            {tierLabel(detail.tier)}
          </span>
          {detail.category && (
            <span className={cn(
              'shrink-0 rounded-sm px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wider',
              palette.chipBg,
              palette.chipText,
            )}>
              {detail.category}
            </span>
          )}
        </div>
        <p className="text-[12.5px] leading-snug text-foreground/80">
          {detail.description}
        </p>
      </div>
    </div>
  );
}

function EmptySkillsState() {
  return (
    <div className="flex flex-col items-center gap-3 rounded-xl border border-dashed border-border/50 bg-secondary/20 px-4 py-8 text-center">
      <div className="inline-flex size-10 items-center justify-center rounded-full bg-mira-blue/10 text-mira-blue">
        <Sparkle className="size-5" />
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

/** Kebab-case icon name → Lucide component. Curated so a user's
 *  frontmatter `icon: shield-check` picks up a matching component
 *  without every lucide icon getting bundled. Unknown names fall
 *  through to Sparkle. */
function iconFor(name: string): React.ComponentType<{ className?: string }> {
  switch (name) {
    case 'shield-check':      return ShieldCheck;
    case 'shield':            return Shield;
    case 'magnifying-glass':  return Search;
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
    case 'settings':          return Cog;
    case 'wrench':            return Wrench;
    case 'rocket':            return Rocket;
    case 'package':           return Package;
    case 'database':          return Database;
    case 'terminal':          return Terminal;
    case 'code':              return Code;
    case 'brackets-curly':    return Braces;
    case 'book':              return Book;
    case 'book-open':         return BookOpen;
    case 'eye':               return Eye;
    case 'scan':              return ScanFace;
    case 'note-pencil':       return NotebookPen;
    case 'sparkle':
    default:                  return Sparkle;
  }
}

/** Pick a default icon key for a skill that didn't specify one. Bundled
 *  skills always ship an explicit `icon:`; this only fires for user-
 *  added ones. We nudge by category — a `git`-category skill without an
 *  icon still reads sensibly as a git-branch. Accepts either the list-
 *  view or the detail payload — both carry `category`. */
function defaultIconKey(s: { category?: string }): string {
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

type Tier = 'bundled' | 'shared' | 'user' | 'project';

function tierLabel(t: Tier): string {
  switch (t) {
    case 'bundled': return 'Bundled';
    case 'shared':  return 'Shared (~/.agents/skills)';
    case 'user':    return 'User (~/.mira/skills)';
    case 'project': return 'Project (.mira/skills)';
  }
}

function tierDotClass(t: Tier): string {
  switch (t) {
    case 'bundled': return 'bg-mira-blue';
    case 'shared':  return 'bg-amber-400';
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
    <TSection
      icon={<Search className="size-3.5" />}
      title="Search & keys"
      description="Third-party API keys tools consume. Stored in your mira.yaml and exported to the process env at startup so tools pick them up transparently."
    >
      {view.keys.map((k) => {
        const meta = KEY_META[k.name];
        const editing = !!draft.keyEditing[k.name];
        const pending = draft.keyValues[k.name];
        const hasStored = k.masked.length > 0;
        return (
          <TRow
            key={k.name}
            title={
              <span className="flex flex-wrap items-baseline gap-x-2">
                <span>{meta?.label ?? k.name}</span>
                <code className="font-mono text-[10.5px] font-normal text-muted-foreground/70">
                  {k.name}
                </code>
              </span>
            }
            description={meta?.help}
            control={
              <span className="flex items-center gap-2">
                {meta?.url && (
                  <a
                    href={meta.url}
                    target="_blank"
                    rel="noreferrer"
                    className="shrink-0 text-[11.5px] text-mira-blue hover:underline"
                  >
                    Get key →
                  </a>
                )}
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
                  <Button variant="outline" size="sm" className="h-8" onClick={() => beginEdit(k.name)}>
                    Add key
                  </Button>
                )}
                {editing && (
                  <>
                    <SectionInput
                      type="password"
                      value={pending ?? ''}
                      onChange={(e) => updateValue(k.name, e.target.value)}
                      placeholder="Paste key…"
                      autoComplete="off"
                      autoFocus
                      spellCheck={false}
                      className="h-8 w-44 font-mono text-[12px]"
                    />
                    <Button variant="outline" size="sm" className="h-8 shrink-0" onClick={() => cancelEdit(k.name)}>
                      Cancel
                    </Button>
                  </>
                )}
              </span>
            }
          />
        );
      })}
    </TSection>
  );
}

/* ---------- section: about ---------- */

function AboutSection({ view }: { view: SettingsView }) {
  return (
    <TSection
      icon={<Info className="size-3.5" />}
      title="About"
      description="Where Mira reads and writes settings on this machine."
    >
      <TRow
        title="Config file"
        control={
          <div className="flex min-w-0 items-center gap-2 rounded-md border border-border/50 bg-background/40 px-2.5 py-1.5 text-[12.5px] sm:w-64">
            <FolderOpen className="size-3.5 shrink-0 text-muted-foreground" />
            <code className="min-w-0 truncate font-mono text-foreground/85">{view.config_path}</code>
          </div>
        }
      />
      <TRow
        title="Status"
        control={
          <div
            className={cn(
              'inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[12.5px]',
              view.configured
                ? 'bg-emerald-500/10 text-emerald-400'
                : 'bg-amber-500/10 text-amber-300',
            )}
          >
            {view.configured ? <Check className="size-3.5" /> : <RotateCw className="size-3.5" />}
            {view.configured ? 'Configured — provider ready' : 'Needs API key'}
          </div>
        }
      />
    </TSection>
  );
}

/* ---------- shared bits: settings layout ---------- */

/**
 * Settings design philosophy, in Mira tokens:
 * - Sections are headed by a quiet muted label row — never a big card
 *   title. Explanatory copy lives on individual rows, not the header.
 * - Rows are title + one-line description on the left, a compact control
 *   flush right. They stack on narrow screens, snap to a two-column grid
 *   past `sm:`.
 * - Rows live in a flat grouped card with hairline dividers; controls
 *   share sizing (inputs/selects h-8, text-[13px]) so every row baselines.
 */
const THEME_CHOICES: { value: ThemePref; label: string; hint: string }[] = [
  { value: 'system', label: 'System', hint: 'Match macOS / your OS' },
  { value: 'light', label: 'Light', hint: 'Bright and paper-like' },
  { value: 'dark', label: 'Dark', hint: 'The classic Mira look' },
];

/** A tiny drawing of the app in one theme, for the theme cards. */
function ThemeSwatch({ theme }: { theme: 'light' | 'dark' }) {
  const dark = theme === 'dark';
  return (
    <span className={cn('flex h-full w-full overflow-hidden', dark ? 'bg-[#202020]' : 'bg-[#eef0f3]')}>
      <span className="flex w-[30%] flex-col gap-1 p-1.5">
        <span className={cn('h-1.5 w-3/4 rounded-full', dark ? 'bg-white/25' : 'bg-black/15')} />
        <span className={cn('h-1.5 w-1/2 rounded-full', dark ? 'bg-white/15' : 'bg-black/10')} />
        <span className={cn('h-1.5 w-2/3 rounded-full', dark ? 'bg-white/15' : 'bg-black/10')} />
      </span>
      <span
        className={cn(
          'my-1.5 mr-1.5 flex flex-1 flex-col gap-1 rounded-[5px] border p-1.5',
          dark ? 'border-white/10 bg-[#0c0c0c]' : 'border-black/10 bg-white',
        )}
      >
        <span className={cn('h-1.5 w-4/5 rounded-full', dark ? 'bg-white/30' : 'bg-black/20')} />
        <span className={cn('h-1.5 w-3/5 rounded-full', dark ? 'bg-white/15' : 'bg-black/10')} />
        <span className="mt-auto h-2.5 w-full rounded-[3px] bg-[#7aa2f7]/70" />
      </span>
    </span>
  );
}

/** A short made-up change, for previewing diff settings. */
const SAMPLE_DIFF = (() => {
  const before = [
    "import { fetchUser } from './api';",
    '',
    'export async function greet(id: string) {',
    '  const user = await fetchUser(id);',
    "  return 'Hello, ' + user.name + '!';",
    '}',
    '',
  ].join('\n');
  const after = [
    "import { fetchUser } from './api';",
    '',
    'export async function greet(id: string, locale = "en") {',
    '  const user = await fetchUser(id);',
    '  if (!user) return null;',
    '  return `Hello, ${user.displayName}!`;',
    '}',
    '',
  ].join('\n');
  try {
    return parseDiffFromFile({ name: 'greet.ts', contents: before }, { name: 'greet.ts', contents: after }, { context: Infinity });
  } catch {
    return null;
  }
})();

function Segmented<T extends string>({
  value, onChange, options,
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: string }[];
}) {
  return (
    <div role="radiogroup" className="inline-flex rounded-lg border border-border/70 bg-fg/[0.03] p-0.5">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          onClick={() => onChange(o.value)}
          className={cn(
            'rounded-md px-2.5 py-1 text-[12px] font-medium transition-colors',
            value === o.value ? 'bg-background text-foreground shadow-sm ring-1 ring-border/70' : 'text-muted-foreground hover:text-foreground',
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

function DiffAppearance() {
  const [layout, setLayout] = useStringPref(PREF_KEYS.diffLayout, 'unified');
  const [wrap, setWrap] = useBoolPref(PREF_KEYS.diffWrap, false);
  const [inline, setInline] = useStringPref(PREF_KEYS.diffInline, 'word');
  const [markers, setMarkers] = useStringPref(PREF_KEYS.diffMarkers, 'bars');
  const [lineNumbers, setLineNumbers] = useBoolPref(PREF_KEYS.diffLineNumbers, true);
  const [tint, setTint] = useBoolPref(PREF_KEYS.diffTint, true);
  const options = useDiffOptions();
  return (
    <TSection
      icon={<GitCompare className="size-3.5" />}
      title="Diffs"
      description="How changes look in the file viewer and the review drawer. Stored in this browser only."
    >
      {SAMPLE_DIFF && (
        <div className="p-3">
          <div className="h-[178px] overflow-hidden rounded-lg border border-border/70">
            <StyledDiffCodeView
              className="h-full min-h-0 overflow-auto"
              items={[{ id: 'preview', type: 'diff', fileDiff: SAMPLE_DIFF, collapsed: false }]}
              options={options}
              renderHeaderFilenameSuffix={() => null}
              renderHeaderPrefix={() => null}
            />
          </div>
        </div>
      )}
      <TRow
        title="Layout"
        description="Unified stacks old and new lines; side by side puts them in two columns."
        control={
          <Segmented
            value={layout === 'split' ? 'split' : 'unified'}
            onChange={setLayout}
            options={[
              { value: 'unified', label: 'Unified' },
              { value: 'split', label: 'Side by side' },
            ]}
          />
        }
      />
      <TRow
        title="Highlight changes within a line"
        description="Marks the exact words or characters that changed, not just the line."
        control={
          <Segmented
            value={(['word', 'char', 'none'] as const).includes(inline as 'word') ? (inline as 'word' | 'char' | 'none') : 'word'}
            onChange={setInline}
            options={[
              { value: 'word', label: 'Words' },
              { value: 'char', label: 'Characters' },
              { value: 'none', label: 'Off' },
            ]}
          />
        }
      />
      <TRow
        title="Change markers"
        description="What sits beside a changed line: a colored bar, or + and − signs."
        control={
          <Segmented
            value={(['bars', 'classic', 'none'] as const).includes(markers as 'bars') ? (markers as 'bars' | 'classic' | 'none') : 'bars'}
            onChange={setMarkers}
            options={[
              { value: 'bars', label: 'Bars' },
              { value: 'classic', label: '+ / −' },
              { value: 'none', label: 'None' },
            ]}
          />
        }
      />
      <TRow
        title="Wrap long lines"
        description="Off: long lines scroll sideways."
        control={<TSwitch checked={wrap} onChange={setWrap} label="Wrap long lines" />}
      />
      <TRow
        title="Line numbers"
        control={<TSwitch checked={lineNumbers} onChange={setLineNumbers} label="Line numbers" />}
      />
      <TRow
        title="Tint changed lines"
        description="Green and red backgrounds on added and removed lines."
        control={<TSwitch checked={tint} onChange={setTint} label="Tint changed lines" />}
      />
    </TSection>
  );
}

function AppearanceSection() {
  const [pref, setPref] = useThemePref();
  const theme = useTheme();
  const accent = useAccent();
  const scale = useUiScale();
  const [reduceMotion, setReduceMotionState] = useBoolPref(PREF_KEYS.reduceMotion, false);
  return (
    <div className="flex flex-col gap-2.5">
      <TSection
        icon={<Palette className="size-3.5" />}
        title="Theme"
        description="Light, dark or your system's, plus accent and size. Stored in this browser only."
      >
        <div className="grid grid-cols-1 gap-3 p-4 sm:grid-cols-3">
          {THEME_CHOICES.map((c) => {
            const on = pref === c.value;
            return (
              <button
                key={c.value}
                type="button"
                role="radio"
                aria-checked={on}
                onClick={() => setPref(c.value)}
                className={cn(
                  'group flex flex-col gap-2.5 rounded-xl border p-2 text-left transition-[border-color,box-shadow]',
                  on
                    ? 'border-mira-blue shadow-[0_0_0_3px_rgb(var(--mira-blue)/0.18)]'
                    : 'border-border hover:border-fg/25',
                )}
              >
                <span className="relative block aspect-[16/10] overflow-hidden rounded-lg border border-border/60">
                  {c.value === 'system' ? (
                    <span className="absolute inset-0 flex">
                      <span className="relative w-1/2 overflow-hidden">
                        <span className="absolute inset-y-0 left-0 w-[200%]"><ThemeSwatch theme="light" /></span>
                      </span>
                      <span className="relative w-1/2 overflow-hidden">
                        <span className="absolute inset-y-0 right-0 w-[200%]"><ThemeSwatch theme="dark" /></span>
                      </span>
                    </span>
                  ) : (
                    <ThemeSwatch theme={c.value} />
                  )}
                </span>
                <span className="flex items-center gap-2 px-1 pb-0.5">
                  <span
                    className={cn(
                      'grid size-4 shrink-0 place-items-center rounded-full border transition-colors',
                      on ? 'border-mira-blue bg-mira-blue' : 'border-fg/25',
                    )}
                  >
                    {on && <span className="size-1.5 rounded-full bg-mira-on-accent" />}
                  </span>
                  <span className="min-w-0">
                    <span className="block text-[13px] font-medium text-foreground">{c.label}</span>
                    <span className="block text-[11.5px] text-muted-foreground">{c.hint}</span>
                  </span>
                </span>
              </button>
            );
          })}
        </div>
        <TRow
          title="Accent color"
          description="Buttons, switches, links and selection."
          control={
            <div role="radiogroup" aria-label="Accent color" className="flex items-center gap-1.5">
              {ACCENTS.map((a) => {
                const on = accent === a;
                const color = ACCENT_SWATCH[a][theme === 'light' ? 1 : 0];
                return (
                  <button
                    key={a}
                    type="button"
                    role="radio"
                    aria-checked={on}
                    aria-label={a}
                    title={a[0].toUpperCase() + a.slice(1)}
                    onClick={() => setAccent(a)}
                    className={cn(
                      'grid size-6 place-items-center rounded-full transition-transform hover:scale-110',
                      on && 'ring-2 ring-offset-2 ring-offset-background',
                    )}
                    style={{ backgroundColor: color, ...(on ? { ['--tw-ring-color' as string]: color } : {}) }}
                  >
                    {on && <Check className="size-3.5 text-white drop-shadow" strokeWidth={3} />}
                  </button>
                );
              })}
            </div>
          }
        />
        <TRow
          title="Interface size"
          description="Scales text and spacing across the whole app."
          control={
            <Segmented
              value={String(scale) as '0.9' | '1' | '1.1' | '1.2'}
              onChange={(v) => setUiScale(Number(v) as UiScale)}
              options={[
                { value: '0.9', label: 'Smaller' },
                { value: '1', label: 'Default' },
                { value: '1.1', label: 'Larger' },
                { value: '1.2', label: 'Largest' },
              ]}
            />
          }
        />
      </TSection>

      <DiffAppearance />

      <TSection icon={<Sparkle className="size-3.5" />} title="Motion">
        <TRow
          title="Reduce motion"
          description="Turn off shimmer, pulses and transitions across the whole app."
          control={
            <TSwitch
              checked={reduceMotion}
              onChange={(v) => {
                setReduceMotionState(v);
                applyReduceMotion();
              }}
              label="Reduce motion"
            />
          }
        />
      </TSection>
    </div>
  );
}

function TSection({
  icon, title, description, action, children,
}: {
  icon?: React.ReactNode;
  title: string;
  description?: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="flex flex-col gap-2.5">
      <div className="flex min-h-7 items-start justify-between gap-4 px-1">
        <div className="min-w-0">
          <h2 className="flex min-h-7 items-center gap-2 text-[13px] font-medium text-muted-foreground">
            {icon}
            {title}
          </h2>
          {description && (
            <p className="mt-1 max-w-xl text-[12px] leading-relaxed text-muted-foreground/75">
              {description}
            </p>
          )}
        </div>
        {action && <div className="flex min-h-7 shrink-0 items-center gap-1.5">{action}</div>}
      </div>
      <div
        className={cn(
          'overflow-hidden rounded-xl border border-border/60 bg-card/40',
          '[&>*+*]:border-t [&>*+*]:border-border/50',
        )}
      >
        {children}
      </div>
    </section>
  );
}

/** One setting: title + description left, compact control right. */
function TRow({
  title, description, status, control, children,
}: {
  title: React.ReactNode;
  description?: React.ReactNode;
  status?: React.ReactNode;
  control?: React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    <div className="px-4 py-3">
      <div className="flex flex-col gap-3 sm:grid sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center sm:gap-6">
        <div className="min-w-0">
          <div className="text-[13.5px] font-medium text-foreground">{title}</div>
          {description && (
            <div className="mt-0.5 max-w-xl text-[12px] leading-relaxed text-muted-foreground/80">
              {description}
            </div>
          )}
          {status && <div className="mt-1 text-[12px] text-muted-foreground">{status}</div>}
        </div>
        {control && (
          <div className="flex min-w-0 shrink-0 items-center gap-2 sm:justify-end">
            {control}
          </div>
        )}
      </div>
      {children}
    </div>
  );
}

/** Muted footnote under a section (global caveats, file paths). */
function TNote({ children }: { children: React.ReactNode }) {
  return (
    <p className="flex items-start gap-1.5 px-1 text-[11.5px] leading-relaxed text-muted-foreground/75">
      {children}
    </p>
  );
}

function TSwitch({ checked, onChange, label }: { checked: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
      className={cn(
        'relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors',
        checked ? 'bg-mira-blue' : 'bg-input',
      )}
    >
      <span
        className={cn(
          'inline-block size-4 rounded-full bg-white shadow transition-transform',
          checked ? 'translate-x-4' : 'translate-x-0.5',
        )}
      />
    </button>
  );
}

type KeyStatus =
  | { kind: 'none' }
  | { kind: 'literal'; masked: string }
  | { kind: 'env'; name: string };

/** Compact API-key control for a row's right column: saved badge, or
 *  an inline paste input. The row itself carries the label + hint. */
function ApiKeyControl({
  status, showInput, value, onChange, onReplace, onCancelReplace,
}: {
  status: KeyStatus;
  showInput: boolean;
  value: string;
  onChange: (v: string) => void;
  onReplace: () => void;
  onCancelReplace: () => void;
}) {
  if (status.kind === 'literal' && !showInput) {
    return (
      <SavedBadge tone="green" icon={<Check className="size-3.5" />} label="Key saved" masked={status.masked} onClick={onReplace} action="Replace" />
    );
  }
  if (status.kind === 'env' && !showInput) {
    return (
      <SavedBadge tone="blue" icon={<Key className="size-3.5" />} label="From env" masked={`$${status.name}`} onClick={onReplace} action="Override" />
    );
  }
  return (
    <div className="flex w-full gap-2 sm:w-64">
      <SectionInput
        type="password"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={status.kind === 'literal' ? `replaces ${status.masked}` : 'sk-…'}
        autoComplete="off"
        spellCheck={false}
        autoFocus={showInput}
        className="h-8 text-[13px]"
      />
      {status.kind !== 'none' && (
        <Button variant="outline" size="sm" onClick={onCancelReplace} type="button" className="h-8 shrink-0">Cancel</Button>
      )}
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

/** Per-provider OAuth affordances — label, icon, description. Kept
 *  as a small config table so adding a third OAuth provider later
 *  needs only one entry, not another wrapper component. */
type OauthSpec = {
  label: string;
  icon: string;
  description: React.ReactNode;
};

const OAUTH_SPECS: Record<string, OauthSpec> = {
  openrouter: {
    label: 'Sign in with OpenRouter',
    icon: openrouterIcon,
    description: (
      <>
        Sign in with your OpenRouter account and we'll receive a key
        automatically — stored in your local <code>mira.yaml</code>.
      </>
    ),
  },
  openai: {
    label: 'Sign in with ChatGPT',
    icon: chatgptIcon,
    description: (
      <>
        Use your ChatGPT Plus / Pro / Team subscription instead of an API key.
        Mira refreshes the underlying token in the background so long sessions
        don't get logged out.
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
function OauthProviderPanel({
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
        } catch { /* transient — keep polling */ }
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
          <SignInPill
            spec={spec}
            status={status}
            onClick={begin}
            label="Sign in again"
          />
        </div>
      ) : (
        <SignInPill
          spec={spec}
          status={status}
          onClick={begin}
          label={spec.label}
        />
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
function SignInPill({
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

/** Settings → External agents: bring Claude Code and Codex chats over.
 *  Scans only when opened — a scan reads every transcript. */
function ImportChatsCard() {
  const [open, setOpen] = useState(false);
  return (
    <div className="mt-8 rounded-2xl border border-border/70 bg-fg/[0.02] p-4">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <div className="text-[14px] font-semibold text-foreground">Bring chats from other agents</div>
          <p className="mt-1 max-w-[60ch] text-[12.5px] leading-relaxed text-muted-foreground">
            Import your Claude Code and Codex conversations, grouped by project. Each opens on the same agent and continues
            where it left off.
          </p>
        </div>
        {!open && (
          <button
            type="button"
            onClick={() => setOpen(true)}
            className="shrink-0 rounded-lg bg-mira-blue px-3 py-1.5 text-[12.5px] font-medium text-white"
          >
            Find chats
          </button>
        )}
      </div>
      <Collapse open={open}>
        <div className="mt-4">
          <ImportChats
            onDone={() => {
              setOpen(false);
              window.dispatchEvent(new Event('mira:sessions-changed'));
            }}
          />
        </div>
      </Collapse>
    </div>
  );
}

