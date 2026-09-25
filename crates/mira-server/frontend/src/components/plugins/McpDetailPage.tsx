import { ArrowLeft, BookOpen, Lock, Pencil, Power, RotateCcw, Terminal, Trash2, Wrench } from 'lucide-react';
import type { McpServerView } from '../../api';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { faviconSrc } from './DiscoverTab';
import { Avatar, Pill } from './shared';
import { StatusBadge, statusText, writeScope, type McpActions } from './McpTab';

function mcpIconSrc(s: McpServerView): string | null {
  if (s.transport === 'http' || s.transport === 'sse') return faviconSrc(s.target);
  return null;
}

export function McpDetailPage({
  server: s,
  busy,
  onBack,
  actions,
}: {
  server: McpServerView;
  busy: string | null;
  onBack: () => void;
  actions: McpActions;
}) {
  const st = s.status.state;
  const editable = writeScope(s) !== null;
  const displayName = (s.scope.kind === 'plugin' ? s.name.split(':').slice(2).join(':') || s.name : s.name)
    .replace(/^\w/, (c) => c.toUpperCase());
  const iconSrc = mcpIconSrc(s);
  const working = busy === `mcp:${s.name}`;

  return (
    <div className="flex flex-col gap-6">
      {/* Breadcrumb */}
      <button
        type="button"
        onClick={onBack}
        className="flex items-center gap-1.5 self-start text-[12.5px] text-muted-foreground transition-colors hover:text-foreground"
      >
        <ArrowLeft className="size-3.5" />
        <span>MCP servers</span>
        <span className="mx-0.5 text-muted-foreground/40">›</span>
        <span className="text-foreground/80">{displayName}</span>
      </button>

      {/* Hero card */}
      <div className="flex gap-5 rounded-2xl border border-border/60 bg-white/[0.035] p-6">
        <div className="relative shrink-0 self-start">
          <Avatar name={displayName} size="lg" src={iconSrc} />
          <StatusBadge status={s.status} size="lg" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2.5">
                <h1 className="text-[22px] font-bold leading-tight tracking-tight">{displayName}</h1>
              </div>
              <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                <Pill className="font-mono uppercase tracking-wider">{s.transport}</Pill>
                <Pill>{s.scope.kind === 'plugin' ? `plugin · ${s.scope.plugin}` : s.scope.kind}</Pill>
                {s.signed_in && <Pill tone="green">signed in</Pill>}
              </div>
              <div
                className={cn(
                  'mt-2 text-[13px]',
                  st === 'failed' ? 'text-destructive' :
                  st === 'connected' ? 'text-muted-foreground/80' :
                  'text-amber-300',
                )}
              >
                {statusText(s)}
              </div>
            </div>

            {/* Actions */}
            <div className="flex shrink-0 flex-wrap items-center gap-2">
              {s.missing_vars.length > 0 && st !== 'connected' && (
                <button
                  type="button"
                  disabled={!!busy}
                  onClick={() => actions.onAddToken(s)}
                  className="rounded-full border border-border/70 px-3.5 py-1.5 text-[12px] text-foreground/90 transition-colors hover:bg-white/[0.06] disabled:opacity-40"
                >
                  Add token
                </button>
              )}
              {st === 'needs_auth' && (
                <button
                  type="button"
                  disabled={!!busy}
                  onClick={() => actions.onSignIn(s.name)}
                  className="rounded-full bg-white px-3.5 py-1.5 text-[12px] font-medium text-black transition-all hover:bg-white/90 active:scale-95 disabled:opacity-40"
                >
                  {working ? 'Working…' : 'Sign in'}
                </button>
              )}
              {st === 'failed' && (
                <button
                  type="button"
                  disabled={!!busy}
                  onClick={() => actions.onReconnect(s.name)}
                  className="inline-flex items-center gap-1.5 rounded-full bg-white px-3.5 py-1.5 text-[12px] font-medium text-black transition-all hover:bg-white/90 active:scale-95 disabled:opacity-40"
                >
                  <RotateCcw className="size-3" />
                  {working ? 'Working…' : 'Retry'}
                </button>
              )}
              {st !== 'disabled' && st !== 'needs_approval' && st !== 'needs_auth' && st !== 'failed' && (
                <Button
                  variant="outline"
                  size="sm"
                  disabled={!!busy}
                  onClick={() => actions.onReconnect(s.name)}
                >
                  <RotateCcw className="size-3.5" />
                  Reconnect
                </Button>
              )}
              <Button
                variant="outline"
                size="sm"
                disabled={!!busy}
                onClick={() => actions.onToggle(s.name, st === 'disabled')}
              >
                <Power className="size-3.5" />
                {st === 'disabled' ? 'Enable' : 'Disable'}
              </Button>
              {editable && (
                <>
                  <Button variant="outline" size="sm" disabled={!!busy} onClick={() => actions.onEdit(s)}>
                    <Pencil className="size-3.5" />
                    Edit
                  </Button>
                  <Button variant="destructive" size="sm" disabled={!!busy} onClick={() => actions.onRemove(s)}>
                    <Trash2 className="size-3.5" />
                    Remove
                  </Button>
                </>
              )}
            </div>
          </div>

          <div className="mt-3 break-all font-mono text-[12px] text-muted-foreground/60">{s.target}</div>
        </div>
      </div>

      {/* Instructions */}
      {s.instructions && (
        <section>
          <SectionH icon={<BookOpen />}>Instructions from the server</SectionH>
          <div className="rounded-xl border border-border/60 bg-white/[0.035] px-4 py-3">
            <p className="whitespace-pre-wrap text-[12.5px] leading-relaxed text-muted-foreground">{s.instructions}</p>
          </div>
        </section>
      )}

      {/* Tools */}
      <section>
        <SectionH icon={<Wrench />}>Tools · {s.tools.length}</SectionH>
        {s.tools.length === 0 ? (
          <div className="rounded-xl border border-border/50 bg-white/[0.035] px-4 py-3 text-[13px] text-muted-foreground">
            {st === 'connected' ? 'This server exposes no tools.' : 'Available once connected.'}
          </div>
        ) : (
          <div className="overflow-hidden rounded-xl border border-border/60 bg-white/[0.035]">
            {s.tools.map((t, i) => (
              <div key={t.name} className={cn('px-4 py-3', i > 0 && 'border-t border-border/40')}>
                <div className="flex items-center gap-2">
                  <code className="font-mono text-[12.5px] text-foreground/90">{t.remote_name}</code>
                  {t.read_only && <Pill tone="green">read-only</Pill>}
                </div>
                {t.description && (
                  <p className="mt-0.5 line-clamp-3 text-[12px] leading-relaxed text-muted-foreground">{t.description}</p>
                )}
              </div>
            ))}
          </div>
        )}
      </section>

      {/* Prompts */}
      {s.prompts.length > 0 && (
        <section>
          <SectionH icon={<Terminal />}>Prompts · run as slash commands</SectionH>
          <div className="overflow-hidden rounded-xl border border-border/60 bg-white/[0.035]">
            {s.prompts.map((p, i) => (
              <div key={p.name} className={cn('px-4 py-3', i > 0 && 'border-t border-border/40')}>
                <code className="font-mono text-[12.5px] text-foreground/90">
                  /mcp__{s.name.replace(/[^A-Za-z0-9-]+/g, '_')}__{p.name}
                </code>
                {p.arguments.length > 0 && (
                  <span className="font-mono text-[12px] text-muted-foreground">
                    {' '}{p.arguments.map((a) => `<${a.name}>`).join(' ')}
                  </span>
                )}
                {p.description && (
                  <p className="mt-0.5 text-[12px] text-muted-foreground">{p.description}</p>
                )}
              </div>
            ))}
          </div>
        </section>
      )}

      {/* Resources */}
      {s.resources.length > 0 && (
        <section>
          <SectionH icon={<BookOpen />}>Resources · {s.resources.length}</SectionH>
          <div className="overflow-hidden rounded-xl border border-border/60 bg-white/[0.035]">
            {s.resources.slice(0, 50).map((r, i) => (
              <div key={r.uri} className={cn('flex items-center gap-3 px-4 py-2.5', i > 0 && 'border-t border-border/40')}>
                <span className="min-w-0 flex-1 truncate text-[13px] text-foreground/90">{r.name}</span>
                <span className="max-w-[240px] shrink-0 truncate font-mono text-[11px] text-muted-foreground/60">{r.uri}</span>
              </div>
            ))}
          </div>
        </section>
      )}

      {/* Information */}
      <section>
        <SectionH>Information</SectionH>
        <div className="overflow-hidden rounded-xl border border-border/60 bg-white/[0.035]">
          {s.server_name && (
            <InfoRow label="Server" value={`${s.server_name}${s.server_version ? ` ${s.server_version}` : ''}`} />
          )}
          {s.can_sign_in && (
            <div className="flex items-center gap-4 border-t border-border/40 px-4 py-3 text-[12.5px] first:border-t-0">
              <span className="w-28 shrink-0 text-muted-foreground">Auth</span>
              <span className="inline-flex items-center gap-1.5 text-foreground/80">
                <Lock className="size-3.5" />
                {s.signed_in ? 'Signed in with OAuth' : 'Not signed in'}
              </span>
            </div>
          )}
          {s.source && <InfoRow label="Defined in" value={s.source} mono />}
          {s.log_path && <InfoRow label="Server log" value={s.log_path} mono />}
        </div>
      </section>

      <p className="pb-4 text-[12px] text-muted-foreground">
        Server tools are available as <code className="rounded bg-white/5 px-1 font-mono text-[11.5px]">mcp__{displayName.replace(/[^A-Za-z0-9-]+/g, '_')}__tool_name</code>.
      </p>
    </div>
  );
}

function SectionH({ icon, children }: { icon?: React.ReactNode; children: React.ReactNode }) {
  return (
    <h2 className="mb-3 flex items-center gap-1.5 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/60 [&_svg]:size-3.5">
      {icon}
      {children}
    </h2>
  );
}

function InfoRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex items-center gap-4 border-t border-border/40 px-4 py-3 text-[12.5px] first:border-t-0">
      <span className="w-28 shrink-0 text-muted-foreground">{label}</span>
      <span
        className={mono ? 'min-w-0 truncate font-mono text-[12px] text-foreground/80' : 'text-foreground/80'}
        title={value}
      >
        {value}
      </span>
    </div>
  );
}
