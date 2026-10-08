/**
 * The engine picker: one place to choose what drives this chat.
 *
 * # Model
 *
 * A session runs on one *engine* — a Mira provider or an external agent —
 * and one model on it. The picker is a rail of engines (providers, then
 * agents) beside the chosen engine's model list, so every choice is the
 * same gesture: point at an engine, pick a model. Picking under an agent
 * switches the chat to that agent; picking under a provider switches it
 * back. There is no separate "stop the agent" control and no tab toggle to
 * find first — the rail *is* the switch, in both directions, at any point
 * in the conversation.
 *
 * The rail only *browses*: hovering or clicking an engine shows its models
 * without changing anything, so exploring never costs the running agent.
 * The change happens on a model pick.
 *
 * # What it knows about an agent that has not started
 *
 * An agent reports its model list only once it runs. The picker shows the
 * list it saw last time (cached per agent) and, failing that, a single
 * "default model" row — picking an agent never waits on a process.
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import { AlertCircle, Check, ChevronDown, Loader2, RefreshCw, Settings2 } from 'lucide-react';
import { listInstanceModels, listModels, type EngineSnapshot, type ModelInfo, type OptionDescriptor } from '../api';
import type { AcpAgentStatus, AcpConfigOption, SessionEngine } from '../types';
import { agentModelChoices, describeAgentStatus, loadAgentCaps } from '../lib/acpAgents';
import {
  CODING_MATCHERS,
  formatCtx,
  groupByVendor,
  loadModelOptions,
  modelDotClass,
  prettyModel,
  saveModelOptions,
} from '../lib/models';
import { AgentIcon, ModelIcon, ProviderIcon } from './AgentIcon';
import { RevertButton } from './AgentSessionActions';
import { ApprovalDialog } from './ApprovalDialog';
import { PrivateText, redactEmails } from './PrivateText';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command';
import { cn } from '@/lib/utils';

/** A rail entry: a provider instance or an agent driver. */
type RailItem =
  | { key: string; kind: 'provider'; instance: string | null; name: string; snapshot?: EngineSnapshot }
  | { key: string; kind: 'agent'; driver: string; name: string; status?: AcpAgentStatus };

type Choice = { id: string; label: string; hint?: string | null; ctx?: number | null };

export type EnginePickerProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** What drives the session now. Null only before the first `ready`. */
  engine: SessionEngine | null;
  /** The session's provider model, used until `engine` arrives. */
  fallbackModel: string;
  /** Configured routing provider, named when the registry lists none. */
  providerName?: string | null;
  /** `GET /api/engines` — native provider instances with their catalogs. */
  engines?: EngineSnapshot[] | null;
  /** Health of every agent this build knows, from the last probe. */
  agents?: AcpAgentStatus[] | null;
  agentsChecking?: boolean;
  onCheckAgents?: () => void;
  /** The running agent's live config options (model list included). */
  agentConfig?: AcpConfigOption[] | null;
  /** The running agent's non-model options, projected for rendering. */
  agentDescriptors?: OptionDescriptor[] | null;
  onPickProvider: (instance: string | null, model: string) => void;
  onPickAgent: (driver: string, model: string | null) => void;
  onSetModelOption: (id: string, value: string) => void;
  onConfigureAgents?: () => void;
  /** Session actions for the running agent. */
  sessionId?: string | null;
  onAgentCompact?: () => void;
  onAgentFork?: () => void;
  onAgentReverted?: () => void;
};

export function EnginePicker(props: EnginePickerProps) {
  const {
    open, onOpenChange, engine, fallbackModel, providerName, engines, agents,
    agentsChecking, onCheckAgents, agentConfig, agentDescriptors,
    onPickProvider, onPickAgent, onSetModelOption, onConfigureAgents,
  } = props;

  const isAgent = engine?.kind === 'agent';
  const currentKey = isAgent
    ? `agent:${engine?.driver ?? ''}`
    : `provider:${engine?.instance ?? ''}`;
  const currentModel = engine?.model ?? (isAgent ? null : fallbackModel);

  // The provider this chat would go back to, remembered while an agent
  // drives so its row keeps its real name and icon.
  const lastProvider = useRef<{ instance: string | null; name: string } | null>(null);
  if (!isAgent && engine) {
    lastProvider.current = {
      instance: engine.instance ?? null,
      name: engine.display_name && engine.display_name !== 'Mira' ? engine.display_name : (providerName ?? 'Mira provider'),
    };
  }

  // ---- the rail ----------------------------------------------------------
  const rail = useMemo<RailItem[]>(() => {
    const items: RailItem[] = [];
    const native = (engines ?? []).filter((e) => e.flavor === 'native' && e.enabled);
    if (native.length > 0) {
      for (const e of native) {
        items.push({ key: `provider:${e.instance}`, kind: 'provider', instance: e.instance, name: e.display_name, snapshot: e });
      }
    }
    // The running provider always has a row, even when the registry is
    // empty or does not list it (a legacy single-provider config).
    const activeProviderKey = `provider:${!isAgent ? (engine?.instance ?? '') : ''}`;
    if (!isAgent && !items.some((i) => i.key === activeProviderKey)) {
      items.unshift({
        key: activeProviderKey,
        kind: 'provider',
        instance: engine?.instance ?? null,
        name: engine?.display_name && engine.display_name !== 'Mira' ? engine.display_name : prettyProvider(providerName),
      });
    }
    if (!items.some((i) => i.kind === 'provider')) {
      const last = lastProvider.current;
      items.push({
        key: `provider:${last?.instance ?? ''}`,
        kind: 'provider',
        instance: last?.instance ?? null,
        name: last?.name ?? prettyProvider(providerName),
      });
    }
    const seen = new Set<string>();
    for (const a of agents ?? []) {
      seen.add(a.kind);
      // Name and on/off come from the agent's engine (Settings → Agents).
      const row = engines?.find((e) => e.flavor === 'external' && e.instance === a.kind);
      if (row && !row.enabled && engine?.driver !== a.kind) continue;
      items.push({ key: `agent:${a.kind}`, kind: 'agent', driver: a.kind, name: row?.display_name || a.display_name, status: a });
    }
    // The session's agent is listed even before the first health probe.
    if (isAgent && engine?.driver && !seen.has(engine.driver)) {
      items.push({ key: `agent:${engine.driver}`, kind: 'agent', driver: engine.driver, name: engine.display_name });
    }
    return items;
  }, [engines, agents, engine, isAgent, providerName]);

  // What the right pane shows. Follows the current engine until the user
  // points elsewhere; reset on every open so it always starts at "here".
  const [browsing, setBrowsing] = useState<string | null>(null);
  useEffect(() => {
    if (open) setBrowsing(null);
  }, [open]);
  const shownKey = browsing ?? currentKey;
  const shown = rail.find((r) => r.key === shownKey) ?? rail.find((r) => r.key === currentKey) ?? rail[0];
  const shownIsCurrent = shown?.key === currentKey;

  // First open with no health data: probe, so agents appear without a
  // trip to Settings.
  const probed = useRef(false);
  useEffect(() => {
    if (open && !probed.current && (agents == null || agents.length === 0 || agents.some((a) => a.kind === 'codex' && !a.models?.length))) {
      probed.current = true;
      onCheckAgents?.();
    }
  }, [open, agents, onCheckAgents]);

  // ---- provider catalogs --------------------------------------------------
  // The registry carries each instance's catalog when it has one; the
  // active provider's is fetched directly as the fallback (and for its
  // capability descriptors).
  const [activeCatalog, setActiveCatalog] = useState<ModelInfo[] | null>(null);
  const [catalogError, setCatalogError] = useState<string | null>(null);
  useEffect(() => {
    if (!open || activeCatalog !== null) return;
    listModels()
      .then((v) => { setActiveCatalog(v.models); setCatalogError(null); })
      .catch((e) => { setActiveCatalog([]); setCatalogError(String((e as Error).message)); });
  }, [open, activeCatalog]);

  // Every other provider's own catalog, fetched when its pane is first
  // shown — each provider lists what *it* serves, not the active one's.
  const [catalogs, setCatalogs] = useState<Record<string, ModelInfo[] | { error: string }>>({});
  const activeInstance = !isAgent ? (engine?.instance ?? null) : null;
  const shownInstance = shown?.kind === 'provider' ? shown.instance : null;
  useEffect(() => {
    if (!open || !shownInstance || shownInstance === activeInstance || catalogs[shownInstance]) return;
    const inst = shownInstance;
    listInstanceModels(inst)
      .then((models) => setCatalogs((c) => ({ ...c, [inst]: models })))
      .catch((e) => setCatalogs((c) => ({ ...c, [inst]: { error: String((e as Error).message) } })));
  }, [open, shownInstance, activeInstance, catalogs]);

  const providerModels = (item: Extract<RailItem, { kind: 'provider' }>): ModelInfo[] | null => {
    const isActive = item.instance === activeInstance || (item.instance === null && !isAgent);
    if (isActive) return activeCatalog;
    const fetched = item.instance ? catalogs[item.instance] : undefined;
    if (Array.isArray(fetched)) return fetched;
    if (fetched) return [];
    if (item.snapshot?.models && item.snapshot.models.length > 0) return item.snapshot.models;
    // The provider the chat would return to, while an agent drives.
    if (isAgent && item.instance === (lastProvider.current?.instance ?? null)) return activeCatalog;
    return null;
  };
  const providerError = (item: Extract<RailItem, { kind: 'provider' }>): string | null => {
    const fetched = item.instance ? catalogs[item.instance] : undefined;
    if (fetched && !Array.isArray(fetched)) return fetched.error;
    return item.instance === activeInstance ? catalogError : null;
  };

  // ---- agent model lists --------------------------------------------------
  const [agentCatalogs, setAgentCatalogs] = useState<Record<string, ModelInfo[] | { error: string }>>({});
  const shownAgent = shown?.kind === 'agent' ? shown.driver : null;
  useEffect(() => {
    if (!open || !shownAgent || agentCatalogs[shownAgent]) return;
    const existing = agents?.find((a) => a.kind === shownAgent)?.models ?? [];
    if (existing.length > 0) return;
    const driver = shownAgent;
    listInstanceModels(driver)
      .then((models) => setAgentCatalogs((c) => ({ ...c, [driver]: models })))
      .catch((e) => setAgentCatalogs((c) => ({ ...c, [driver]: { error: String((e as Error).message) } })));
  }, [open, shownAgent, agents, agentCatalogs]);

  const agentChoices = (driver: string): { choices: Choice[]; live: boolean; error?: string } => {
    const live = isAgent && engine?.driver === driver ? agentModelChoices(agentConfig) : [];
    if (live.length > 0) return { choices: live.map((c) => ({ id: c.value, label: c.label })), live: true };
    const fetched = agentCatalogs[driver];
    if (Array.isArray(fetched) && fetched.length > 0) {
      return { choices: fetched.map((m) => ({ id: m.id, label: m.display_name || prettyModel(m.id) })), live: true };
    }
    if (fetched && !Array.isArray(fetched)) return { choices: [], live: false, error: fetched.error };
    const catalog = agents?.find((a) => a.kind === driver)?.models ?? [];
    if (catalog.length > 0) return { choices: catalog.map((m) => ({ id: m.value, label: m.label })), live: true };
    const cached = agentModelChoices(loadAgentCaps(driver).config);
    return { choices: cached.map((c) => ({ id: c.value, label: c.label })), live: false };
  };

  // ---- per-model options (provider engines) -------------------------------
  const currentInfo = !isAgent ? (activeCatalog?.find((m) => m.id === currentModel) ?? null) : null;
  const descriptors: OptionDescriptor[] = isAgent
    ? (agentDescriptors ?? [])
    : (currentInfo?.capabilities?.option_descriptors ?? []);
  const optionsInstance = !isAgent ? (engine?.instance ?? null) : null;
  const [options, setOptions] = useState<Record<string, string>>(() => loadModelOptions(currentModel ?? '', optionsInstance));
  useEffect(() => {
    // On a provider model change, re-apply that model's remembered options
    // so the next turn runs with them rather than the previous model's.
    if (isAgent || !currentModel) return;
    const next = loadModelOptions(currentModel, optionsInstance);
    setOptions(next);
    for (const [id, value] of Object.entries(next)) onSetModelOption(id, value);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentModel, isAgent, optionsInstance]);
  function pickOption(id: string, value: string) {
    const next = { ...options, [id]: value };
    setOptions(next);
    if (!isAgent && currentModel) saveModelOptions(currentModel, next, optionsInstance);
    onSetModelOption(id, value);
  }

  // ---- picking ---------------------------------------------------------------
  const [blocked, setBlocked] = useState<AcpAgentStatus | null>(null);
  function commit(item: RailItem, model: string | null) {
    if (item.kind === 'provider') {
      if (!model) return;
      onPickProvider(item.instance, model);
    } else {
      if (item.status && item.status.state.state === 'not_found') {
        setBlocked(item.status);
        return;
      }
      onPickAgent(item.driver, model);
    }
    onOpenChange(false);
  }

  // ---- trigger ---------------------------------------------------------------
  const triggerLabel = isAgent
    ? (currentModel ? prettyModel(currentModel) : 'Default model')
    : prettyModel(currentModel) || 'Choose model';
  const optionSummary = summarizeOptions(descriptors, options);

  return (
    <>
      <Popover open={open} onOpenChange={onOpenChange}>
        <PopoverTrigger asChild>
          <button
            type="button"
            title={isAgent ? `${engine?.display_name}${currentModel ? ` · ${currentModel}` : ''}` : currentModel ?? ''}
            className="inline-flex min-w-0 max-w-[22rem] items-center gap-1.5 rounded-md px-2 py-1.5 text-foreground transition-colors hover:bg-fg/[0.04] touch:py-2.5"
          >
            {/* Every child sits in the same 16px line box — icon, labels,
                status and chevron — so they share one centre line. The
                labels used to sit on a shared *baseline* inside a centred
                row, which left the smaller glyphs (and the chevron beside
                them) visibly off-centre. */}
            <span className="grid size-4 shrink-0 place-items-center"><EngineMark engine={engine} model={currentModel} /></span>
            <span className="inline-flex h-4 min-w-0 items-center gap-1.5">
              {isAgent ? (
                <>
                  <span className="shrink-0 text-[12.5px] font-semibold leading-none">{engine?.display_name}</span>
                  <span className="min-w-0 truncate text-[12.5px] font-medium leading-none text-muted-foreground">
                    {triggerLabel}
                  </span>
                </>
              ) : (
                <span className="min-w-0 truncate text-[12.5px] font-semibold leading-none">{triggerLabel}</span>
              )}
              {optionSummary && (
                <span className="shrink-0 text-[11.5px] font-medium leading-none text-muted-foreground/80 max-md:hidden">{optionSummary}</span>
              )}
            </span>
            <EngineStatusGlyph engine={engine} />
            <span className="grid size-4 shrink-0 place-items-center">
              <ChevronDown className="size-3 text-muted-foreground/60" />
            </span>
          </button>
        </PopoverTrigger>
        <PopoverContent
          align="start"
          side="top"
          sideOffset={8}
          className="w-[36rem] max-w-[calc(100vw-2rem)] overflow-hidden rounded-xl border border-border/60 bg-popover/95 p-0 backdrop-blur"
        >
          <div className="grid h-[26rem] grid-cols-[12.5rem_minmax(0,1fr)]">
            {/* Rail */}
            <div className="flex min-h-0 flex-col border-r border-border/50 bg-fg/[0.015]">
              <div className="min-h-0 flex-1 overflow-y-auto p-1.5">
                <RailSection label="Providers" />
                {rail.filter((r) => r.kind === 'provider').map((r) => (
                  <RailRow
                    key={r.key}
                    item={r}
                    current={r.key === currentKey}
                    shown={r.key === shown?.key}
                    onShow={() => setBrowsing(r.key)}
                  />
                ))}
                <RailSection
                  label="External agents"
                  action={onCheckAgents && (
                    <button
                      type="button"
                      onClick={onCheckAgents}
                      title="Check which agents are installed"
                      className="rounded p-0.5 text-muted-foreground/60 transition-colors hover:text-foreground"
                    >
                      <RefreshCw className={cn('size-3', agentsChecking && 'animate-spin')} />
                    </button>
                  )}
                />
                {rail.filter((r) => r.kind === 'agent').map((r) => (
                  <RailRow
                    key={r.key}
                    item={r}
                    current={r.key === currentKey}
                    shown={r.key === shown?.key}
                    onShow={() => setBrowsing(r.key)}
                  />
                ))}
                {!rail.some((r) => r.kind === 'agent') && (
                  <div className="px-2 py-1.5 text-[11.5px] text-muted-foreground/60">
                    {agentsChecking ? 'Looking for installed agents…' : 'No external agents found.'}
                  </div>
                )}
              </div>
              {onConfigureAgents && (
                <button
                  type="button"
                  onClick={() => { onOpenChange(false); onConfigureAgents(); }}
                  className="flex items-center gap-1.5 border-t border-border/50 px-3 py-2 text-left text-[11.5px] text-muted-foreground transition-colors hover:text-foreground"
                >
                  <Settings2 className="size-3.5" />
                  Set up external agents
                </button>
              )}
            </div>

            {/* Pane */}
            <div className="flex min-h-0 min-w-0 flex-col">
              {shown && (
                <PaneHeader
                  item={shown}
                  current={shownIsCurrent}
                  engine={engine}
                />
              )}
              <div className="min-h-0 flex-1">
                {shown?.kind === 'agent' && shown.status?.state.state === 'not_found' ? (
                  <NotInstalled status={shown.status} onSetUp={() => { onOpenChange(false); onConfigureAgents?.(); }} />
                ) : shown ? (
                  <ModelList
                    key={shown.key}
                    item={shown}
                    currentModel={shownIsCurrent ? currentModel : null}
                    isCurrent={shownIsCurrent}
                    providerModels={shown.kind === 'provider' ? providerModels(shown) : null}
                    agentChoices={shown.kind === 'agent' ? agentChoices(shown.driver) : null}
                    catalogError={shown.kind === 'provider' ? providerError(shown) : null}
                    onPick={(model) => commit(shown, model)}
                  />
                ) : null}
              </div>

              {/* Options and session actions belong to the running engine,
                  so they show only while its pane is open. */}
              {shownIsCurrent && (descriptors.length > 0 || isAgent) && (
                <div className="shrink-0 border-t border-border/50 px-2 py-1.5">
                  {descriptors.map((d) => (
                    <div key={d.id} className="flex items-center gap-2 rounded-md px-1.5 py-1">
                      <span className="w-24 shrink-0 text-[12px] text-foreground/75">{d.label}</span>
                      <span className="flex min-w-0 flex-1 justify-end">
                        <OptionControl
                          descriptor={d}
                          value={options[d.id] ?? currentOptionValue(d, agentConfig)}
                          onChange={(v) => pickOption(d.id, v)}
                        />
                      </span>
                    </div>
                  ))}
                  {isAgent && (
                    <div className="flex items-center gap-1.5 px-1.5 pt-1">
                      <span className="mr-auto text-[11px] text-muted-foreground/60">This chat's agent session</span>
                      {props.onAgentCompact && (
                        <SmallButton onClick={() => { props.onAgentCompact?.(); onOpenChange(false); }} title="Ask the agent to compact its context">
                          Compact
                        </SmallButton>
                      )}
                      {props.onAgentFork && engine?.driver === 'claude-code' && (
                        <SmallButton onClick={() => { props.onAgentFork?.(); onOpenChange(false); }} title="Continue this history under a new agent session id">
                          Fork
                        </SmallButton>
                      )}
                      <RevertButton sessionId={props.sessionId} onReverted={props.onAgentReverted} />
                    </div>
                  )}
                </div>
              )}
              {!shownIsCurrent && shown && (
                <div className="shrink-0 border-t border-border/50 px-3 py-2 text-[11px] leading-snug text-muted-foreground/70">
                  {shown.kind === 'agent'
                    ? `Pick a model to hand this chat to ${shown.name}. It uses its own login and billing, and gets the conversation so far.`
                    : `Pick a model to continue this chat on ${shown.name}${isAgent ? ` — ${engine?.display_name}'s turns come with it` : ''}.`}
                </div>
              )}
            </div>
          </div>
        </PopoverContent>
      </Popover>
      {blocked && (
        <ApprovalDialog
          request={{
            title: `${blocked.display_name} isn't installed`,
            source: { label: blocked.display_name, detail: 'agent' },
            body: (
              <div className="space-y-2">
                <p><PrivateText text={describeAgentStatus(blocked).text} /></p>
                <p className="text-muted-foreground/70">Settings → Agents has the install steps.</p>
              </div>
            ),
            choices: [
              { id: 'cancel', label: 'Cancel' },
              { id: 'settings', label: 'Open Settings', primary: true },
            ],
            onDismiss: () => setBlocked(null),
            onChoose: (id: string) => {
              setBlocked(null);
              if (id === 'settings') onConfigureAgents?.();
            },
          }}
        />
      )}
    </>
  );
}

/* ---------- pieces ---------- */

/** The session's engine mark, for the trigger. */
export function EngineMark({ engine, model, size = 'xs' }: { engine: SessionEngine | null; model?: string | null; size?: 'xs' | 'sm' }) {
  if (engine?.kind === 'agent' && engine.driver) {
    return <AgentIcon kind={engine.driver} name={engine.display_name} size={size} tile={false} />;
  }
  return <ModelIcon model={model ?? engine?.model} size={size} />;
}

/** A spinner while an agent starts, a red mark if it failed. */
function EngineStatusGlyph({ engine }: { engine: SessionEngine | null }) {
  if (engine?.kind !== 'agent') return null;
  if (engine.status === 'starting') {
    return <Loader2 className="size-3 shrink-0 animate-spin text-muted-foreground" aria-label="Starting" />;
  }
  if (engine.status === 'error') {
    return (
      <span title={engine.error ?? 'Failed to start'}>
        <AlertCircle className="size-3.5 shrink-0 text-destructive" aria-label="Failed to start" />
      </span>
    );
  }
  return null;
}

function RailSection({ label, action }: { label: string; action?: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between px-2 pb-1 pt-2 first:pt-1">
      <span className="text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground/55">{label}</span>
      {action}
    </div>
  );
}

function RailRow({
  item, current, shown, onShow,
}: {
  item: RailItem;
  current: boolean;
  shown: boolean;
  onShow: () => void;
}) {
  const health =
    item.kind === 'agent' && item.status
      ? describeAgentStatus(item.status).tone
      : item.kind === 'provider' && item.snapshot && item.snapshot.state.state !== 'ready'
        ? 'error'
        : null;
  return (
    <button
      type="button"
      // Click, not hover: moving the pointer from the rail to the list
      // crosses other rows, and hover-to-show flipped the pane to
      // whichever engine the pointer passed on the way.
      onClick={onShow}
      className={cn(
        'relative flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors',
        shown ? 'bg-fg/[0.08]' : 'hover:bg-fg/[0.04]',
      )}
    >
      {item.kind === 'agent' ? (
        <AgentIcon kind={item.driver} name={item.name} size="xs" tile={false} />
      ) : (
        <ProviderIcon instance={item.instance ?? item.name} name={item.name} model={item.snapshot?.default_model} size="xs" />
      )}
      <span className={cn('min-w-0 flex-1 truncate text-[12.5px] leading-4', current ? 'font-semibold text-foreground' : 'text-foreground/85')}>
        {item.name}
      </span>
      {current ? (
        <Check className="size-3.5 shrink-0 text-mira-blue" aria-label="In use" />
      ) : health === 'error' ? (
        <span className="size-1.5 shrink-0 rounded-full bg-muted-foreground/35" title="Not ready" />
      ) : null}
    </button>
  );
}

function PaneHeader({ item, current, engine }: { item: RailItem; current: boolean; engine: SessionEngine | null }) {
  let sub: string;
  if (item.kind === 'agent') {
    if (current && engine?.status === 'starting') sub = 'Starting…';
    else if (current && engine?.status === 'error') sub = engine.error ?? 'Failed to start';
    else if (current && engine?.status === 'idle') sub = 'Starts with your next message';
    else sub = item.status ? describeAgentStatus(item.status).text : 'External agent';
  } else {
    const s = item.snapshot;
    sub = s?.auth ?? (s && s.state.state !== 'ready' && 'reason' in s.state ? s.state.reason : 'Mira provider');
  }
  return (
    <div className="flex shrink-0 items-center gap-2 border-b border-border/50 px-3 py-2.5">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="truncate text-[13px] font-semibold text-foreground">{item.name}</span>
          {current && (
            <span className="shrink-0 rounded-full bg-mira-blue/15 px-1.5 py-px text-[10px] font-medium text-mira-blue">
              this chat
            </span>
          )}
        </div>
        <div
          className={cn(
            'truncate text-[11px]',
            current && engine?.status === 'error' ? 'text-destructive/90' : 'text-muted-foreground/70',
          )}
          title={redactEmails(sub)}
        >
          <PrivateText text={sub} />
        </div>
      </div>
    </div>
  );
}

function NotInstalled({ status, onSetUp }: { status: AcpAgentStatus; onSetUp: () => void }) {
  return (
    <div className="flex h-full flex-col items-start justify-center gap-2 px-5 text-[12.5px] text-muted-foreground">
      <p className="text-foreground/85"><PrivateText text={describeAgentStatus(status).text} />.</p>
      {status.install_hint && (
        <code className="max-w-full truncate rounded-md border border-border/60 bg-muted/40 px-2 py-1 font-mono text-[11.5px] text-foreground/85">
          {status.install_hint}
        </code>
      )}
      <button
        type="button"
        onClick={onSetUp}
        className="mt-1 rounded-lg bg-mira-blue/90 px-3 py-1 text-[12px] font-medium text-white transition-colors hover:bg-mira-blue"
      >
        Set up {status.display_name}
      </button>
    </div>
  );
}

function ModelList({
  item, currentModel, isCurrent, providerModels, agentChoices, catalogError, onPick,
}: {
  item: RailItem;
  currentModel: string | null;
  isCurrent: boolean;
  providerModels: ModelInfo[] | null;
  agentChoices: { choices: Choice[]; live: boolean; error?: string } | null;
  catalogError: string | null;
  onPick: (model: string | null) => void;
}) {
  const [query, setQuery] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    // Typing is the fastest way through a long catalog.
    const t = setTimeout(() => inputRef.current?.focus(), 0);
    return () => clearTimeout(t);
  }, []);

  const loading = item.kind === 'provider' && providerModels === null;

  const sections = useMemo(() => {
    if (item.kind === 'agent') {
      const choices = agentChoices?.choices ?? [];
      return choices.length > 0 ? [{ name: agentChoices?.live ? 'Models' : 'Models (last seen)', choices }] : [];
    }
    const models = providerModels ?? [];
    const out: { name: string; choices: Choice[] }[] = [];
    const toChoice = (m: ModelInfo): Choice => ({
      id: m.id,
      label: m.display_name || prettyModel(m.id),
      hint: m.id,
      ctx: m.context_length,
    });
    if (!query.trim()) {
      const picked = new Set<string>();
      const suggested: Choice[] = [];
      for (const match of CODING_MATCHERS) {
        const hit = models.find((m) => !picked.has(m.id) && match(m.id));
        if (hit) { picked.add(hit.id); suggested.push(toChoice(hit)); }
      }
      if (suggested.length > 1 && models.length > 12) out.push({ name: 'Best for coding', choices: suggested });
    }
    const groups = groupByVendor(models);
    if (groups.length <= 1) {
      out.push({ name: 'Models', choices: models.map(toChoice) });
    } else {
      for (const g of groups) out.push({ name: g.name, choices: g.models.map(toChoice) });
    }
    return out;
  }, [item, providerModels, agentChoices, query]);

  return (
    <Command shouldFilter className="flex h-full flex-col bg-transparent">
      <CommandInput
        ref={inputRef}
        value={query}
        onValueChange={setQuery}
        placeholder={loading ? 'Loading models…' : item.kind === 'agent' ? 'Search, or type a model id…' : 'Search models, or type an id…'}
      />
      <CommandList className="max-h-none min-h-0 flex-1 overflow-y-auto">
        {catalogError && <div className="px-3 py-2 text-[11.5px] text-destructive">{catalogError}</div>}
        {item.kind === 'agent' && agentChoices?.error && <div className="px-3 py-2 text-[11.5px] text-destructive">{agentChoices.error}</div>}
        {loading && (
          <div className="flex items-center gap-2 px-3 py-3 text-[12px] text-muted-foreground">
            <Loader2 className="size-3.5 animate-spin" /> Fetching models…
          </div>
        )}
        {item.kind === 'agent' && !query.trim() && (
          <CommandGroup>
            <CommandItem value="__default__" onSelect={() => onPick(null)} className="flex items-center gap-2">
              <AgentIcon kind={item.driver} name={item.name} size="xs" tile={false} />
              <div className="min-w-0 flex-1">
                <div className="text-[13px]">
                  Default model
                  {isCurrent && !currentModel && <Check className="ml-1.5 inline size-3 text-mira-blue" />}
                </div>
                <div className="text-[10.5px] text-muted-foreground/70">
                  Whatever {item.name} is set to use
                </div>
              </div>
            </CommandItem>
          </CommandGroup>
        )}
        {sections.map((sec) => (
          <CommandGroup key={sec.name} heading={sec.name}>
            {sec.choices.map((c) => (
              <CommandItem
                key={`${sec.name}:${c.id}`}
                value={`${c.id} ${c.label}`}
                onSelect={() => onPick(c.id)}
                className="flex items-center gap-2"
              >
                <span className={cn('size-2 shrink-0 rounded-full', modelDotClass(c.id))} />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5 truncate text-[13px]">
                    {c.label}
                    {c.id === currentModel && <Check className="size-3 shrink-0 text-mira-blue" />}
                  </div>
                  {c.hint && c.hint !== c.label && (
                    <div className="truncate font-mono text-[10.5px] text-muted-foreground/65">{c.hint}</div>
                  )}
                </div>
                {c.ctx ? (
                  <span className="shrink-0 rounded-full border border-mira-blue/25 bg-mira-blue/[0.09] px-1.5 py-0.5 font-mono text-[10.5px] text-mira-blue">
                    {formatCtx(c.ctx)}
                  </span>
                ) : null}
              </CommandItem>
            ))}
          </CommandGroup>
        ))}
        {item.kind === 'agent' && !agentChoices?.live && !query.trim() && (
          <div className="px-3 pb-2 pt-1 text-[11px] text-muted-foreground/60">
            {(agentChoices?.choices.length ?? 0) > 0
              ? 'The live list replaces this once the agent is running.'
              : 'Its model list appears once it has started.'}
          </div>
        )}
        {query.trim() && (
          <CommandGroup heading="Custom">
            <CommandItem value={`__use__${query}`} onSelect={() => onPick(query.trim())}>
              <span className="text-[12.5px] text-muted-foreground">Use</span>
              <code className="ml-1 rounded border border-mira-tool/20 bg-mira-tool/[0.12] px-1.5 py-0.5 font-mono text-xs text-mira-tool">
                {query.trim()}
              </code>
            </CommandItem>
          </CommandGroup>
        )}
        <CommandEmpty>No matches</CommandEmpty>
      </CommandList>
    </Command>
  );
}

function SmallButton({ children, onClick, title }: { children: React.ReactNode; onClick: () => void; title?: string }) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      className="rounded-lg border border-border/80 px-2 py-1 text-[11.5px] text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground"
    >
      {children}
    </button>
  );
}

/** The agent's own current value for an option, when it reports one. */
function currentOptionValue(d: OptionDescriptor, config: AcpConfigOption[] | null | undefined): string {
  const live = config?.find((o) => o.id === d.id)?.current;
  if (live) return live;
  return d.type === 'select' ? (d.options[0]?.value ?? '') : '';
}

/**
 * The short string shown after the model name on the trigger: whatever the
 * user changed away from a default. A "Fast" tier is worth the space; an
 * `auto` effort is not.
 */
function summarizeOptions(descriptors: OptionDescriptor[], options: Record<string, string>): string {
  for (const d of descriptors) {
    if (d.type !== 'select') continue;
    const match = d.options.find((o) => o.value === options[d.id]);
    if (match && match.value !== 'off' && match.value !== 'auto') return match.label;
  }
  for (const d of descriptors) {
    if (d.type === 'boolean' && options[d.id]) return d.label;
  }
  return '';
}

/**
 * One advertised option: `select` descriptors get a dropdown, `boolean`
 * ones a switch — so "Fast" renders right whether a provider sends it as a
 * service-tier select or a fast-mode flag.
 */
function OptionControl({
  descriptor, value, onChange,
}: {
  descriptor: OptionDescriptor;
  value: string;
  onChange: (value: string) => void;
}) {
  if (descriptor.type === 'boolean') {
    const on = value === (descriptor.on_value ?? 'on');
    return (
      <button
        type="button"
        role="switch"
        aria-checked={on}
        aria-label={descriptor.label}
        onClick={() => onChange(on ? '' : (descriptor.on_value ?? 'on'))}
        className={cn(
          'h-5 w-9 shrink-0 rounded-full p-0.5 transition-colors',
          on ? 'bg-foreground/85' : 'bg-muted-foreground/25 hover:bg-muted-foreground/40',
        )}
      >
        <span className={cn('block size-4 rounded-full bg-background shadow transition-transform', on ? 'translate-x-4' : 'translate-x-0')} />
      </button>
    );
  }
  const current = descriptor.options.find((o) => o.value === value) ?? descriptor.options[0];
  if (!current) return null;
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="inline-flex max-w-[11rem] items-center gap-1 rounded-md px-1.5 py-0.5 text-[12.5px] text-foreground/85 transition-colors hover:bg-fg/[0.05]"
        >
          <span className="truncate">{current.label}</span>
          <ChevronDown className="size-3 shrink-0 text-muted-foreground/60" />
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" sideOffset={6} className="w-56 rounded-lg border border-popover-border bg-popover p-1">
        {descriptor.options.map((o) => (
          <button
            key={o.value}
            type="button"
            role="menuitemradio"
            aria-checked={o.value === current.value}
            onClick={() => onChange(o.value)}
            className={cn(
              'flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-fg/[0.05]',
              o.value === current.value ? 'text-foreground' : 'text-muted-foreground',
            )}
          >
            <span className="min-w-0 flex-1">
              <span className="block truncate text-[12.5px] font-medium">{o.label}</span>
              {o.hint && <span className="mt-0.5 block truncate text-[11px] text-muted-foreground/75">{o.hint}</span>}
            </span>
            {o.value === current.value && <Check className="size-3.5 shrink-0" />}
          </button>
        ))}
      </PopoverContent>
    </Popover>
  );
}

/** A provider slug as a name, for when nothing better is known. */
function prettyProvider(slug: string | null | undefined): string {
  if (!slug) return 'Mira provider';
  const known: Record<string, string> = {
    openrouter: 'OpenRouter', openai: 'OpenAI', anthropic: 'Anthropic', google: 'Google',
    gemini: 'Gemini', groq: 'Groq', xai: 'xAI', deepseek: 'DeepSeek', mistral: 'Mistral',
    ollama: 'Ollama', together: 'Together', fireworks: 'Fireworks', bedrock: 'Bedrock',
  };
  return known[slug.toLowerCase()] ?? slug.charAt(0).toUpperCase() + slug.slice(1);
}
