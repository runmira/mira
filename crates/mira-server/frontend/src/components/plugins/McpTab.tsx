import {
  Eye,
  LogIn,
  LogOut,
  Pencil,
  Plug,
  Plus,
  Power,
  RotateCcw,
  ShieldAlert,
  Trash2,
} from 'lucide-react';
import type { McpListView, McpServerView, McpStatus, WriteScope } from '../../api';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { matchesQuery } from './DiscoverTab';
import { EmptyState, Pill, RowMenu, plural } from './shared';

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
    case 'failed':
      return `Failed: ${s.status.message}`;
  }
}

export function StatusDot({ status }: { status: McpStatus }) {
  const cls = {
    connected:       'bg-emerald-400 shadow-[0_0_6px_1px_theme(colors.emerald.400/40%)]',
    connecting:      'bg-mira-blue animate-pulse',
    needs_auth:      'bg-amber-400',
    needs_approval:  'bg-amber-400',
    rejected:        'bg-white/20',
    disabled:        'bg-white/20',
    failed:          'bg-destructive',
  }[status.state];
  return <span className={cn('mt-[7px] size-2 shrink-0 rounded-full', cls)} />;
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
      <div className="flex items-center justify-between gap-3">
        <p className="text-[13px] text-muted-foreground">
          Servers connect in the background; their tools reach the agent as{' '}
          <code className="font-mono text-[12px]">mcp__server__tool</code>. Changes apply right away.
        </p>
        <Button size="sm" className="h-8 shrink-0 gap-1.5" onClick={actions.onAdd}>
          <Plus className="size-3.5" />
          Add server
        </Button>
      </div>

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
            <Button size="sm" className="mt-1 gap-1.5" onClick={actions.onAdd}>
              <Plus className="size-3.5" /> Add server
            </Button>
          }
        />
      ) : (
        GROUPS.map((g) => {
          const rows = servers.filter((s) => s.scope.kind === g.key);
          if (rows.length === 0) return null;
          return (
            <section key={g.key}>
              <div className="mb-2.5 flex items-baseline gap-2">
                <h3 className="text-[13px] font-semibold">{g.title}</h3>
                <span className="text-[12px] text-muted-foreground">{g.hint}</span>
              </div>
              <ul className="flex flex-col divide-y divide-border/50 overflow-hidden rounded-2xl border border-border/60 bg-white/[0.02]">
                {rows.map((s) => (
                  <ServerRow key={s.name} s={s} busy={busy} actions={actions} />
                ))}
              </ul>
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

function ServerRow({ s, busy, actions }: { s: McpServerView; busy: string | null; actions: McpActions }) {
  const st = s.status.state;
  const editable = writeScope(s) !== null;
  const primary =
    st === 'needs_auth'
      ? { label: 'Sign in', icon: <LogIn />, run: () => actions.onSignIn(s.name) }
      : st === 'needs_approval'
        ? { label: 'Approve', icon: <ShieldAlert />, run: () => actions.onApprove(s.name, true) }
        : st === 'failed'
          ? { label: 'Retry', icon: <RotateCcw />, run: () => actions.onReconnect(s.name) }
          : st === 'disabled'
            ? { label: 'Enable', icon: <Power />, run: () => actions.onToggle(s.name, true) }
            : null;
  const menu = [
    { label: 'Details', icon: <Eye />, onSelect: () => actions.onOpen(s) },
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
    ...(editable
      ? [
          { label: 'Edit',   icon: <Pencil />, onSelect: () => actions.onEdit(s) },
          { label: 'Remove', icon: <Trash2 />, danger: true, onSelect: () => actions.onRemove(s) },
        ]
      : []),
  ];
  return (
    <li
      className="flex cursor-pointer items-start gap-3 px-4 py-3.5 transition-colors hover:bg-white/[0.03]"
      onClick={() => actions.onOpen(s)}
    >
      <StatusDot status={s.status} />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-[13.5px] font-semibold">
            {s.scope.kind === 'plugin' ? s.name.split(':').slice(2).join(':') || s.name : s.name}
          </span>
          <Pill className="font-mono uppercase tracking-wider">{s.transport}</Pill>
          {s.scope.kind === 'plugin' && <Pill tone="purple">{s.scope.plugin}</Pill>}
          {s.signed_in && <Pill tone="green">signed in</Pill>}
        </div>
        <div className="mt-0.5 truncate font-mono text-[11.5px] text-muted-foreground/70" title={s.target}>
          {s.target}
        </div>
        <div
          className={cn(
            'mt-1 line-clamp-2 text-[12.5px]',
            st === 'failed'    ? 'text-destructive'      :
            st === 'connected' ? 'text-muted-foreground' :
                                 'text-amber-300',
          )}
        >
          {statusText(s)}
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-1.5" onClick={(e) => e.stopPropagation()}>
        {primary && (
          <Button size="sm" variant="outline" className="h-7 gap-1.5 rounded-lg" disabled={!!busy} onClick={primary.run}>
            <span className="[&_svg]:size-3.5">{primary.icon}</span>
            {busy === `mcp:${s.name}` ? 'Working…' : primary.label}
          </Button>
        )}
        <RowMenu items={menu} />
      </div>
    </li>
  );
}
