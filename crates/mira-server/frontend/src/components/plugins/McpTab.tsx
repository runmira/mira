import {
  KeyRound,
  LogIn,
  LogOut,
  Pencil,
  Plug,
  Power,
  RotateCcw,
  ShieldAlert,
  Trash2,
} from 'lucide-react';
import type { McpListView, McpServerView, McpStatus, ToolLoading, WriteScope } from '../../api';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { faviconSrc, matchesQuery } from './DiscoverTab';
import { Avatar, EmptyState, Pill, RowMenu, plural } from './shared';

export type McpActions = {
  onAdd: () => void;
  onEdit: (s: McpServerView) => void;
  onRemove: (s: McpServerView) => void;
  onOpen: (s: McpServerView) => void;
  onReconnect: (name: string) => void;
  onToggle: (name: string, enabled: boolean) => void;
  onApprove: (name: string, approve: boolean) => void;
  onSignIn: (name: string) => void;
  onSignOut: (name: string) => void;
  /** Paste values for the server's unset `${VAR}`s (tokens). */
  onAddToken: (s: McpServerView) => void;
  onToolEnabled: (tool: string, enabled: boolean) => void;
  onToolLoading: (mode: ToolLoading) => void;
};

export function statusText(s: McpServerView): string {
  switch (s.status.state) {
    case 'connected': {
      const parts = [plural(s.tools.length, 'tool')];
      if (s.prompts.length) parts.push(plural(s.prompts.length, 'prompt'));
      if (s.resources.length) parts.push(plural(s.resources.length, 'resource'));
      return `Connected · ${parts.join(' · ')}`;
    }
    case 'connecting':
      return 'Connecting…';
    case 'needs_auth':
      return 'Needs sign-in';
    case 'needs_approval':
      return 'Waiting for your approval';
    case 'rejected':
      return 'Rejected';
    case 'disabled':
      return 'Disabled';
    case 'needs_setup':
      return `Needs ${s.status.variables.join(', ')}: add it with “Add token”`;
    case 'failed':
      return `Failed: ${s.status.message}`;
  }
}

const STATUS_CLS: Record<McpStatus['state'], string> = {
  connected:      'bg-emerald-400 shadow-[0_0_6px_1px_theme(colors.emerald.400/40%)]',
  connecting:     'bg-mira-blue animate-pulse',
  needs_auth:     'bg-amber-400',
  needs_approval: 'bg-amber-400',
  rejected:       'bg-white/20',
  disabled:       'bg-white/20',
  needs_setup:    'bg-amber-400',
  failed:         'bg-destructive',
};

/** Inline dot — kept for backward compat in detail page header etc. */
export function StatusDot({ status }: { status: McpStatus }) {
  return <span className={cn('mt-[7px] size-2 shrink-0 rounded-full', STATUS_CLS[status.state])} />;
}

/** Absolute badge overlay — place inside a `relative` wrapper around the Avatar. */
export function StatusBadge({ status, size = 'md' }: { status: McpStatus; size?: 'md' | 'lg' }) {
  return (
    <span
      className={cn(
        'absolute rounded-full ring-2 ring-[hsl(var(--background))]',
        size === 'lg' ? 'bottom-0.5 left-0.5 size-3' : 'bottom-0 left-0 size-2.5',
        STATUS_CLS[status.state],
      )}
    />
  );
}

export function writeScope(s: McpServerView): WriteScope | null {
  return s.scope.kind === 'plugin' ? null : s.scope.kind;
}

const GROUPS: { key: McpServerView['scope']['kind']; title: string; hint: string }[] = [
  { key: 'local',   title: 'Local',   hint: 'This project, only you' },
  { key: 'project', title: 'Project', hint: '.mcp.json, shared with the repo' },
  { key: 'user',    title: 'User',    hint: 'All your projects' },
  { key: 'plugin',  title: 'Plugins', hint: 'From installed plugins' },
];

export function McpTab({
  data,
  query,
  busy,
  actions,
}: {
  data: McpListView;
  query: string;
  busy: string | null;
  actions: McpActions;
}) {
  const servers = data.servers.filter((s) => matchesQuery(query, s.name, s.target, s.server_name));
  const pending = data.servers.filter((s) => s.status.state === 'needs_approval');

  return (
    <div className="flex flex-col gap-5">
      <div className="flex items-start justify-between gap-3">
        <p className="text-[12.5px] leading-relaxed text-muted-foreground/70">
          Tools appear as <code className="rounded bg-white/5 px-1 font-mono text-[11.5px]">mcp__server__tool</code> — servers connect in the background and changes apply right away.
        </p>
        <button
          type="button"
          onClick={actions.onAdd}
          className="shrink-0 rounded-full bg-white px-4 py-1.5 text-[12px] font-medium text-black transition-all hover:bg-white/90 active:scale-95"
        >
          + Add server
        </button>
      </div>

      {data.servers.length > 0 && (
        <ToolLoadingPicker
          mode={data.tool_loading}
          onDemand={data.tools_on_demand}
          total={data.servers.reduce((n, s) => n + (s.status.state === 'connected' ? s.tools.filter((t) => t.enabled).length : 0), 0)}
          busy={!!busy}
          onChange={actions.onToolLoading}
        />
      )}

      {pending.length > 0 && (
        <div className="rounded-2xl border border-amber-500/25 bg-amber-500/[0.05] p-4">
          <div className="flex items-center gap-2 text-[13.5px] font-semibold text-amber-300">
            <ShieldAlert className="size-4" />
            This project wants to run {plural(pending.length, 'MCP server')}
          </div>
          <p className="mt-1 text-[12.5px] text-amber-100/60">
            Defined in the repo, so review what each one runs before allowing it. Approval is
            remembered until the definition changes.
          </p>
          <ul className="mt-3 flex flex-col gap-2">
            {pending.map((s) => (
              <li key={s.name} className="flex items-center gap-3 rounded-xl bg-black/20 px-3 py-2.5">
                <div className="min-w-0 flex-1">
                  <div className="text-[13px] font-medium">{s.name}</div>
                  <div className="truncate font-mono text-[11.5px] text-muted-foreground" title={s.target}>
                    {s.transport} · {s.target}
                  </div>
                </div>
                <Button variant="ghost" size="sm" className="h-7" disabled={!!busy} onClick={() => actions.onApprove(s.name, false)}>
                  Reject
                </Button>
                <Button size="sm" className="h-7" disabled={!!busy} onClick={() => actions.onApprove(s.name, true)}>
                  Approve
                </Button>
              </li>
            ))}
          </ul>
        </div>
      )}

      {data.servers.length === 0 ? (
        <EmptyState
          icon={<Plug />}
          title="No MCP servers"
          body="Connect tools like GitHub, Linear, Sentry or a database. Add a server, or install a plugin that ships one."
          action={
            <button
              type="button"
              onClick={actions.onAdd}
              className="mt-2 rounded-full bg-white px-4 py-1.5 text-[12px] font-medium text-black transition-all hover:bg-white/90 active:scale-95"
            >
              + Add server
            </button>
          }
        />
      ) : (
        GROUPS.map((g) => {
          const cards = servers.filter((s) => s.scope.kind === g.key);
          if (cards.length === 0) return null;
          return (
            <section key={g.key}>
              <div className="mb-2.5 flex items-baseline gap-2">
                <h3 className="text-[13px] font-semibold">{g.title}</h3>
                <span className="text-[12px] text-muted-foreground">{g.hint}</span>
              </div>
              <div className="grid gap-2.5 md:grid-cols-2">
                {cards.map((s) => (
                  <ServerCard key={s.name} s={s} busy={busy} actions={actions} />
                ))}
              </div>
            </section>
          );
        })
      )}

      {data.problems.filter((p) => !p.message.startsWith('overridden')).length > 0 && (
        <section>
          <h3 className="mb-2 text-[13px] font-semibold">Couldn't load</h3>
          <ul className="flex flex-col gap-1.5">
            {data.problems
              .filter((p) => !p.message.startsWith('overridden'))
              .map((p, i) => (
                <li key={i} className="rounded-xl border border-destructive/25 bg-destructive/5 px-3 py-2.5 text-[12.5px]">
                  <span className="font-mono text-[11.5px] text-muted-foreground">{p.source}</span>
                  {p.server && <span className="text-foreground"> · {p.server}</span>}
                  <div className="text-destructive">{p.message}</div>
                </li>
              ))}
          </ul>
        </section>
      )}
    </div>
  );
}

function ServerCard({ s, busy, actions }: { s: McpServerView; busy: string | null; actions: McpActions }) {
  const st = s.status.state;
  const editable = writeScope(s) !== null;
  const displayName = (s.scope.kind === 'plugin' ? s.name.split(':').slice(2).join(':') || s.name : s.name)
    .replace(/^\w/, (c) => c.toUpperCase());
  const iconSrc = (s.transport === 'http' || s.transport === 'sse') ? faviconSrc(s.target) : null;
  const working = busy === `mcp:${s.name}`;

  const needsToken = s.missing_vars.length > 0 && st !== 'connected';
  const primary =
    st === 'needs_setup'
      ? { label: 'Add token', run: () => actions.onAddToken(s) }
      : st === 'needs_auth'
      ? { label: 'Sign in', run: () => actions.onSignIn(s.name) }
      : st === 'needs_approval'
        ? { label: 'Approve', run: () => actions.onApprove(s.name, true) }
        : st === 'failed'
          ? { label: 'Retry', run: () => actions.onReconnect(s.name) }
          : st === 'disabled'
            ? { label: 'Enable', run: () => actions.onToggle(s.name, true) }
            : null;

  const menu = [
    ...(st !== 'disabled' && st !== 'needs_approval' && st !== 'rejected'
      ? [{ label: 'Reconnect', icon: <RotateCcw />, onSelect: () => actions.onReconnect(s.name) }]
      : []),
    st === 'disabled'
      ? { label: 'Enable',  icon: <Power />, onSelect: () => actions.onToggle(s.name, true) }
      : { label: 'Disable', icon: <Power />, onSelect: () => actions.onToggle(s.name, false) },
    ...(s.can_sign_in && s.signed_in
      ? [{ label: 'Sign out', icon: <LogOut />, onSelect: () => actions.onSignOut(s.name) }]
      : []),
    ...(s.can_sign_in && !s.signed_in && st !== 'needs_auth'
      ? [{ label: 'Sign in', icon: <LogIn />, onSelect: () => actions.onSignIn(s.name) }]
      : []),
    ...(s.missing_vars.length > 0
      ? [{ label: 'Add token', icon: <KeyRound />, onSelect: () => actions.onAddToken(s) }]
      : []),
    ...(editable
      ? [
          { label: 'Edit',   icon: <Pencil />, onSelect: () => actions.onEdit(s) },
          { label: 'Remove', icon: <Trash2 />, danger: true, onSelect: () => actions.onRemove(s) },
        ]
      : []),
  ];

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => actions.onOpen(s)}
      onKeyDown={(e) => e.key === 'Enter' && actions.onOpen(s)}
      className="group flex cursor-pointer gap-3.5 rounded-2xl border border-border/60 bg-white/[0.08] p-4 text-left shadow-sm shadow-black/30 transition-all hover:border-border/80 hover:bg-white/[0.11] hover:shadow-md hover:shadow-black/40"
    >
      <div className="relative shrink-0 self-start">
        <Avatar name={displayName} src={iconSrc} size="md" />
        <StatusBadge status={s.status} size="md" />
      </div>

      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        {/* Title row */}
        <div className="flex items-start justify-between gap-2">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="truncate text-[13.5px] font-semibold leading-tight">{displayName}</span>
            </div>
            <div className="mt-0.5 flex flex-wrap items-center gap-1">
              <Pill className="font-mono text-[10.5px] uppercase tracking-wider">{s.transport}</Pill>
              {s.scope.kind === 'plugin' && <Pill tone="purple">{s.scope.plugin}</Pill>}
              {s.signed_in && <Pill tone="green">signed in</Pill>}
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-1" onClick={(e) => e.stopPropagation()}>
            <RowMenu items={menu} />
          </div>
        </div>

        {/* Target */}
        <div className="truncate font-mono text-[11px] text-muted-foreground/55" title={s.target}>
          {s.target}
        </div>

        {/* Status */}
        <div className={cn(
          'text-[12px]',
          st === 'failed'    ? 'text-destructive' :
          st === 'connected' ? 'text-muted-foreground/70' :
                               'text-amber-300',
        )}>
          {statusText(s)}
        </div>

        {/* Primary action */}
        {primary && (
          <div className="mt-0.5 flex items-center gap-1.5" onClick={(e) => e.stopPropagation()}>
            <button
              type="button"
              disabled={!!busy}
              onClick={primary.run}
              className="rounded-full bg-white px-3 py-1 text-[11.5px] font-medium text-black transition-all hover:bg-white/90 active:scale-95 disabled:opacity-40"
            >
              {working ? 'Working…' : primary.label}
            </button>
            {needsToken && st === 'needs_auth' && (
              <button
                type="button"
                disabled={!!busy}
                onClick={() => actions.onAddToken(s)}
                title="Use a token instead of signing in"
                className="rounded-full border border-border/70 px-3 py-1 text-[11.5px] text-foreground/85 transition-colors hover:bg-white/[0.06] disabled:opacity-40"
              >
                Add token
              </button>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

const LOADING: { mode: ToolLoading; label: string }[] = [
  { mode: 'auto', label: 'Auto' },
  { mode: 'all', label: 'All up front' },
  { mode: 'on_demand', label: 'On demand' },
];

/** How MCP tools reach the model. Every definition costs tokens on every
 *  turn, and long tool lists confuse smaller models, so past a point the
 *  model looks tools up instead. */
function ToolLoadingPicker({
  mode,
  onDemand,
  total,
  busy,
  onChange,
}: {
  mode: ToolLoading;
  onDemand: boolean;
  total: number;
  busy: boolean;
  onChange: (m: ToolLoading) => void;
}) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-border/50 bg-white/[0.035] px-4 py-3">
      <div className="min-w-0">
        <div className="text-[13px] font-medium">Tool loading</div>
        <div className="text-[12px] text-muted-foreground/80">
          {plural(total, 'tool')} enabled ·{' '}
          {onDemand
            ? 'the model finds tools with search_mcp_tools when it needs them'
            : 'every tool is sent to the model each turn'}
          {mode === 'auto' && ' · switches to on demand above 30 tools'}
        </div>
      </div>
      <div className="inline-flex shrink-0 rounded-full border border-border/60 bg-black/20 p-0.5">
        {LOADING.map((o) => (
          <button
            key={o.mode}
            type="button"
            disabled={busy}
            onClick={() => o.mode !== mode && onChange(o.mode)}
            className={cn(
              'rounded-full px-3 py-1 text-[11.5px] transition-colors disabled:opacity-50',
              o.mode === mode ? 'bg-white text-black' : 'text-muted-foreground hover:text-foreground',
            )}
          >
            {o.label}
          </button>
        ))}
      </div>
    </div>
  );
}
