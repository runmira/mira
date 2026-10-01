/**
 * The external-agent list.
 *
 * # Why this is its own section, not part of Provider
 *
 * A provider is *Mira calling a model endpoint*: one base URL, one key, one
 * model list, billed to the user's Mira account. An external ACP agent is a
 * separate runtime with its own login, its own billing, and its own model
 * catalogue — and the key point is that **the user's Provider key does not
 * configure it**. Codex may be signed in through a ChatGPT subscription,
 * Claude Code through a Pro/Max login. Filing them under one list would imply
 * a link that does not exist, and the likeliest outcome would be someone
 * pasting an OpenAI key into a Codex field and concluding Codex is broken.
 *
 * So the two are siblings, cross-referenced from both sides, and this list
 * says plainly that keys here do not come from Provider.
 *
 * # Why the config lives client-side
 *
 * An agent's binary path, config dir and launch args describe how to run a
 * third-party binary *on this machine* — a local concern, unlike provider
 * credentials which the server needs in order to call an API. So it persists
 * in `localStorage` and travels with the start request, rather than joining
 * the server's settings blob.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { motion } from 'framer-motion';
import { AlertCircle, Check, Loader2, RefreshCw, Terminal } from 'lucide-react';
import type { AcpAgentStatus } from '../../types';
import { AgentHistoryImport } from './AgentHistoryImport';
import { AgentIcon } from '../AgentIcon';
import { PrivateText, redactEmails } from '../PrivateText';
import {
  describeAgentStatus,
  loadInstanceConfigs,
  parseArgs,
  parseEnv,
  saveInstanceConfigs,
  type AcpInstanceConfig,
  type AgentHealthTone,
} from '../../lib/acpAgents';

// Kept here so existing imports keep working; the implementation lives in
// lib/acpAgents.ts, shared with the sidebar.
export type { AcpInstanceConfig } from '../../lib/acpAgents';
export { loadInstanceConfigs, saveInstanceConfigs };

type Tone = AgentHealthTone;

/**
 * Health only. Whether the user has switched the agent on is applied
 * separately, in the row: the two used to be conflated and every agent read
 * "Disabled" regardless of what was actually installed.
 */


function Switch({
  on,
  onChange,
  label,
}: {
  on: boolean;
  onChange: (v: boolean) => void;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      onClick={() => onChange(!on)}
      className={[
        'relative h-[20px] w-[34px] shrink-0 rounded-full transition-colors duration-200',
        'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-mira-blue/50',
        on ? 'bg-mira-blue' : 'bg-muted-foreground/25 hover:bg-muted-foreground/35',
      ].join(' ')}
    >
      <motion.span
        layout
        transition={{ type: 'spring', stiffness: 500, damping: 34 }}
        className={[
          'absolute top-[2px] size-4 rounded-full bg-white shadow-sm',
          on ? 'left-[16px]' : 'left-[2px]',
        ].join(' ')}
      />
    </button>
  );
}

const inputCls =
  'w-full rounded-lg border border-border/80 bg-background/60 px-3 py-2 text-[12.5px] ' +
  'outline-none transition-colors placeholder:text-muted-foreground/40 ' +
  'focus:border-mira-blue/50 focus:bg-background';

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="block">
      <div className="text-[12px] font-medium text-foreground/85">{label}</div>
      {hint && (
        <div className="mt-0.5 text-[11px] leading-relaxed text-muted-foreground/65">
          {hint}
        </div>
      )}
      <div className="mt-1.5">{children}</div>
    </label>
  );
}

function Note({ text, tone }: { text: string; tone: Tone }) {
  const Icon = tone === 'error' ? AlertCircle : tone === 'warn' ? Loader2 : Check;
  return (
    <div
      className={[
        'flex items-start gap-1.5 text-[11.5px] leading-relaxed',
        tone === 'error' ? 'text-destructive/90' : 'text-muted-foreground/75',
      ].join(' ')}
    >
      <Icon className="mt-[1px] size-3 shrink-0" />
      <span>{text}</span>
    </div>
  );
}

/**
 * The three facts that make agents make sense, in the order a user meets
 * them. Previously the page opened on a live-session banner ("… is driving
 * this session" + Stop) that mixed a chat's state into a settings page and
 * explained nothing about how the pieces fit.
 */
function HowAgentsWork() {
  const steps: { title: string; body: React.ReactNode }[] = [
    {
      title: 'Install and sign in',
      body: 'Each agent runs its own CLI with its own login and billing. Your provider keys are never used for it.',
    },
    {
      title: 'Pick it in the chat',
      body: (
        <>
          Open the model picker in the composer and choose a model under{' '}
          <span className="text-foreground/85">External agents</span>. It takes over at once.
        </>
      ),
    },
    {
      title: 'Switch whenever you like',
      body: 'Pick a provider model to switch back. The conversation carries across both ways, and new chats start on whatever you used last.',
    },
  ];
  return (
    <ol className="grid gap-2 sm:grid-cols-3">
      {steps.map((s, i) => (
        <li key={s.title} className="rounded-xl border border-border/70 bg-muted/20 px-3.5 py-3">
          <div className="flex items-center gap-2">
            <span className="grid size-5 shrink-0 place-items-center rounded-full bg-mira-blue/15 text-[10.5px] font-semibold text-mira-blue">
              {i + 1}
            </span>
            <span className="text-[12.5px] font-medium text-foreground/90">{s.title}</span>
          </div>
          <p className="mt-1.5 text-[11.5px] leading-relaxed text-muted-foreground/80">{s.body}</p>
        </li>
      ))}
    </ol>
  );
}

/**
 * One agent's full detail: status, config, start, history import.
 *
 * Rendered in the right pane for whichever agent the left list selects —
 * never inline under a row. Health rows and config used to share an
 * accordion card, which meant configuring anything required expanding rows
 * one by one; the list is for scanning, the pane is for acting.
 */
function AgentDetail({
  agent,
  cfg,
  update,
  onStart,
}: {
  agent: AcpAgentStatus;
  cfg: AcpInstanceConfig;
  update: (kind: string, patch: Partial<AcpInstanceConfig>) => void;
  onStart: (kind: string, cfg: AcpInstanceConfig, resume?: string | null) => void;
}) {
  return (
    <div className="space-y-3.5">
  <Field
    label="Display name"
    hint="Shown in place of the agent's own name."
  >
    <input
      className={inputCls}
      value={cfg.displayName ?? ''}
      placeholder={agent.display_name}
      onChange={(e) =>
        update(agent.kind, {
          displayName: e.target.value || undefined,
        })
      }
    />
  </Field>

  <Field
    label="Binary path"
    hint={`Leave empty to use the agent on PATH. Resolved command: ${agent.launch}`}
  >
    <input
      className={inputCls + ' font-mono text-[11.5px]'}
      value={cfg.binaryPath ?? ''}
      placeholder="(from PATH)"
      onChange={(e) =>
        update(agent.kind, {
          binaryPath: e.target.value || undefined,
        })
      }
    />
  </Field>

  <Field
    label="Config directory"
    hint="Overrides the agent's home, which keeps credentials and config separate per instance."
  >
    <input
      className={inputCls + ' font-mono text-[11.5px]'}
      value={cfg.homePath ?? ''}
      placeholder="~/.claude"
      onChange={(e) =>
        update(agent.kind, { homePath: e.target.value || undefined })
      }
    />
  </Field>

  {agent.kind === 'claude-code' && (
    <Field
      label="Effort"
      hint="How hard Claude tries per turn. Leave unset for its default."
    >
      <select
        className={inputCls + ' text-[12.5px]'}
        value={cfg.effort ?? ''}
        onChange={(e) =>
          update(agent.kind, { effort: e.target.value || undefined })
        }
      >
        <option value="">Agent default</option>
        {['low', 'medium', 'high', 'xhigh', 'max'].map((l) => (
          <option key={l} value={l}>{l}</option>
        ))}
      </select>
    </Field>
  )}
  {agent.kind === 'claude-code' && (
    <Field
      label="Setting sources"
      hint="Which settings layers the CLI loads (comma-separated: user, project, local)."
    >
      <input
        className={inputCls + ' font-mono text-[11.5px]'}
        value={cfg.settingSources ?? ''}
        placeholder="user,project,local"
        onChange={(e) =>
          update(agent.kind, { settingSources: e.target.value || undefined })
        }
      />
    </Field>
  )}
  <Field
    label="Launch arguments"
    hint="Appended verbatim. Quotes are respected, so --model 'gpt 5' works."
  >
    <input
      className={inputCls + ' font-mono text-[11.5px]'}
      value={cfg.launchArgs ?? ''}
      onChange={(e) =>
        update(agent.kind, { launchArgs: e.target.value || undefined })
      }
    />
  </Field>

  <Field
    label="Environment"
    hint="One KEY=value per line. Blank lines and # comments are ignored."
  >
    <textarea
      className={inputCls + ' h-20 resize-y font-mono text-[11.5px]'}
      value={cfg.env ?? ''}
      onChange={(e) =>
        update(agent.kind, { env: e.target.value || undefined })
      }
    />
  </Field>

  <Field
    label="API key"
    hint="Only for agents that take one. Stored in this browser and never logged — the resolved command is shown redacted."
  >
    <input
      className={inputCls + ' font-mono text-[11.5px]'}
      type="password"
      value={cfg.apiKey ?? ''}
      onChange={(e) =>
        update(agent.kind, { apiKey: e.target.value || undefined })
      }
    />
  </Field>

  {(agent.state.state === 'not_found' || !cfg.enabled) && (
    <div className="flex items-center gap-3 pt-0.5">
      {agent.state.state === 'not_found' && (
        <span className="text-[11.5px] text-destructive/85">
          {agent.cli_installed && agent.install_hint ? (
            <>
              The agent is installed, but Mira needs its ACP adapter:{' '}
              <code className="font-mono">{agent.install_hint}</code>
            </>
          ) : agent.install_hint ? (
            <>
              Install it with <code className="font-mono">{agent.install_hint}</code>, then press Check.
            </>
          ) : (
            'Install the agent, then press Check.'
          )}
        </span>
      )}
      {!cfg.enabled && (
        <span className="text-[11.5px] text-muted-foreground/70">
          Hidden from the model picker. Switch it on to use it.
        </span>
      )}
    </div>
  )}
  {agent.kind === 'claude-code' && (
    <div className="border-t border-border/60 pt-2.5">
      <AgentHistoryImport
        onResume={(sid) =>
          onStart(
            'claude-code',
            {
              ...cfg,
              launchArgs: cfg.launchArgs ?? '',
              env: cfg.env ?? '',
            },
            sid,
          )
        }
      />
    </div>
  )}
    </div>
  );
}

export function AcpAgentsSection({
  agents,
  onRefresh,
  refreshing,
  onStart,
  activeKind,
  error,
}: {
  agents: AcpAgentStatus[];
  onRefresh: () => void;
  refreshing: boolean;
  /** Only for opening an imported agent session in the current chat. */
  onStart: (kind: string, cfg: AcpInstanceConfig, resume?: string | null) => void;
  /** The agent the current chat runs on, badged in the list. */
  activeKind: string | null;
  error?: string | null;
}) {
  const [configs, setConfigs] = useState<Record<string, AcpInstanceConfig>>(() =>
    loadInstanceConfigs(),
  );
  const [selected, setSelected] = useState<string | null>(null);
  // Health is probed on demand, but landing on this page IS the demand —
  // showing "no agents yet" until a manual click is a dead end, especially
  // on a first visit when the list has never populated. The WS client
  // queues pre-open sends, so firing on mount is safe either way.
  const probed = useRef(false);
  useEffect(() => {
    if (agents.length === 0 && !refreshing && !probed.current) {
      probed.current = true;
      onRefresh();
    }
  }, [agents.length, refreshing, onRefresh]);

  const update = useCallback((kind: string, patch: Partial<AcpInstanceConfig>) => {
    setConfigs((prev) => {
      const next = {
        ...prev,
        [kind]: { ...(prev[kind] ?? { enabled: true }), ...patch },
      };
      saveInstanceConfigs(next);
      return next;
    });
  }, []);

  const rows = useMemo(
    () =>
      agents.map((a) => ({ agent: a, cfg: configs[a.kind] ?? { enabled: true } })),
    [agents, configs],
  );

  if (agents.length === 0) {
    return (
      <div className="flex items-center gap-2 rounded-xl border border-dashed border-border/70 px-4 py-6 text-[12.5px] text-muted-foreground/70">
        {refreshing ? (
          <>
            <Loader2 className="size-3.5 animate-spin" />
            Checking for installed agents…
          </>
        ) : (
          <div className="space-y-2">
            <div className="flex items-center gap-2 text-foreground/80">
              <Terminal className="size-3.5" />
              No external agents on this computer yet.
            </div>
            <p className="max-w-[60ch] leading-relaxed">
              Mira can hand a chat to a coding agent you already use, signed in with your own
              account. Install one, then check again:
            </p>
            <ul className="space-y-1 font-mono text-[11.5px] text-foreground/75">
              <li>npm i -g @anthropic-ai/claude-code <span className="font-sans text-muted-foreground/60">— Claude Code</span></li>
              <li>npm i -g @openai/codex <span className="font-sans text-muted-foreground/60">— Codex</span></li>
            </ul>
            <button
              type="button"
              onClick={onRefresh}
              className="underline underline-offset-2 hover:text-foreground"
            >
              Check again
            </button>
          </div>
        )}
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-start justify-between gap-6">
        <div className="text-[12.5px] leading-relaxed text-muted-foreground">
          Coding agents you have installed — Claude Code, Codex and others —
          can drive a Mira chat instead of one of your providers. This page
          is only setup; choosing one happens in the chat.
        </div>
        <button
          type="button"
          onClick={onRefresh}
          disabled={refreshing}
          className={[
            'flex shrink-0 items-center gap-1.5 rounded-lg border border-border/80 px-2.5 py-1.5',
            'text-[11.5px] text-muted-foreground transition-colors',
            'hover:bg-muted/50 hover:text-foreground disabled:opacity-50',
          ].join(' ')}
        >
          <RefreshCw className={`size-3 ${refreshing ? 'animate-spin' : ''}`} />
          {refreshing ? 'Checking' : 'Check'}
        </button>
      </div>

      <HowAgentsWork />

      {error && <Note text={error} tone="error" />}

      {/* Master-detail: the list scans, the pane acts. Selecting never
          navigates away, and configuring never requires expanding rows one
          by one inside a scrolling list. Selection is a neutral grey fill;
          blue is reserved for "driving", so the two never compete. */}
      {/* Wide tables scroll; they never clip. The floor keeps both columns
          usable on narrow panes, and inline columns (not a generated class)
          so the layout cannot silently collapse to one column. */}
      <div className="overflow-x-auto pb-1">
      <div
        className="grid min-w-[36rem] items-start gap-5 isolate"
        style={{ gridTemplateColumns: '15rem minmax(0, 1fr)' }}
      >
        <div className="max-h-[calc(100dvh-20rem)] min-w-0 overflow-y-auto">
          <div className="px-2.5 pb-1.5 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground/60">
            Agents
          </div>
          <div className="grid gap-0.5" role="listbox" aria-label="Agents">
            {rows.map(({ agent, cfg }) => {
              const st = describeAgentStatus(agent);
              const isActive = activeKind === agent.kind;
              const isSel = (selected ?? activeKind ?? rows[0]?.agent.kind) === agent.kind;
              const name = cfg.displayName || agent.display_name;
              return (
                <button
                  key={agent.kind}
                  type="button"
                  role="option"
                  aria-selected={isSel}
                  onClick={() => setSelected(agent.kind)}
                  className={[
                    'flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-left transition-colors',
                    isSel ? 'bg-white/[0.08]' : 'hover:bg-white/[0.04]',
                    !cfg.enabled && 'opacity-55',
                  ].join(' ')}
                >
                  <AgentIcon kind={agent.kind} name={name} size="md" />
                  <span className="min-w-0 flex-1">
                    <span className="flex items-center gap-1.5">
                      <span className="truncate text-[13px] font-medium text-foreground/90">
                        {name}
                      </span>
                      {isActive && (
                        <span className="shrink-0 rounded-full bg-mira-blue/15 px-1.5 py-px text-[10px] font-medium text-mira-blue" title="The current chat runs on this agent">
                          this chat
                        </span>
                      )}
                    </span>
                    <span
                      className="mt-0.5 flex items-start gap-1.5 text-[11px] text-muted-foreground/65"
                      title={redactEmails(agent.state.state === 'failed' ? agent.state.reason : st.text)}
                    >
                      <span
                        className={[
                          'mt-1 size-1.5 shrink-0 rounded-full',
                          !cfg.enabled
                            ? 'bg-muted-foreground/25'
                            : st.tone === 'ok'
                              ? 'bg-emerald-500'
                              : st.tone === 'error'
                                ? 'bg-destructive'
                                : 'bg-muted-foreground/40',
                        ].join(' ')}
                      />
                      {/* Short in the list, full in the detail pane: emails
                          and adapter hints wrap here instead of clipping
                          mid-address, and the complete text is one click
                          away on the right. */}
                      <span className="line-clamp-2 min-w-0 break-words">
                        {!cfg.enabled && agent.state.state !== 'failed'
                          ? 'Hidden'
                          : agent.state.state === 'ready'
                            ? (agent.version ? `v${agent.version}` : 'Ready')
                            : agent.state.state === 'failed'
                              ? 'Failed'
                              : agent.cli_installed
                                ? 'Adapter missing'
                                : 'Not installed'}
                      </span>
                    </span>
                  </span>
                </button>
              );
            })}
          </div>
        </div>

        {(() => {
          const sel =
            rows.find((r) => r.agent.kind === (selected ?? activeKind ?? rows[0]?.agent.kind)) ??
            rows[0];
          if (!sel) return null;
          const { agent, cfg } = sel;
          const name = cfg.displayName || agent.display_name;
          const isActive = activeKind === agent.kind;
          const st = describeAgentStatus(agent);
          return (
            <div className="flex max-h-[calc(100dvh-20rem)] min-h-[16rem] min-w-0 flex-col overflow-hidden rounded-xl border border-border/60 bg-background/40">
              {/* Pinned header: name, version, driving badge, toggle at far
                  left of its cluster. Sticky so the field list scrolls
                  underneath while the tab itself never scrolls away. */}
              <div className="sticky top-0 z-10 flex shrink-0 items-center gap-2 border-b border-border/60 bg-background/95 px-4 py-3 backdrop-blur">
                <Switch
                  on={cfg.enabled}
                  onChange={(v) => update(agent.kind, { enabled: v })}
                  label={`Enable ${name}`}
                />
                <span className="truncate text-[14px] font-semibold text-foreground">
                  {name}
                </span>
                {agent.version && (
                  <span className="shrink-0 text-[11.5px] tabular-nums text-muted-foreground/60">
                    v{agent.version}
                  </span>
                )}
                {isActive && (
                  <span className="shrink-0 rounded-full bg-mira-blue/15 px-1.5 py-px text-[10.5px] font-medium text-mira-blue">
                    this chat
                  </span>
                )}
                <span className="ml-auto shrink-0 text-[11px] text-muted-foreground/60">
                  {cfg.enabled ? 'In picker' : 'Hidden'}
                </span>
              </div>
              <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4">
                <div
                  className="mb-3 truncate text-[12px] text-muted-foreground/75"
                  title={redactEmails(agent.state.state === 'failed' ? agent.state.reason : st.text)}
                >
                  <PrivateText text={agent.state.state === 'failed' ? agent.state.reason : st.text} />
                </div>
                <AgentDetail
                  agent={agent}
                  cfg={cfg}
                  update={update}
                  onStart={onStart}
                />
              </div>
            </div>
          );
        })()}
      </div>
      </div>

    </div>
  );
}

export { parseArgs, parseEnv };
