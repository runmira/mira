import { Info, Puzzle, RotateCcw, Trash2 } from 'lucide-react';
import type { InstalledPluginView, PluginsOverview } from '../../api';
import { faviconSrc, matchesQuery } from './DiscoverTab';
import { Avatar, EmptyState, Pill, RowMenu, Switch, plural, relativeTime } from './shared';

export function InstalledTab({
  data,
  query,
  busy,
  onOpen,
  onToggle,
  onUpdate,
  onUninstall,
  onBrowse,
}: {
  data: PluginsOverview;
  query: string;
  busy: string | null;
  onOpen: (id: string) => void;
  onToggle: (id: string, enabled: boolean) => void;
  onUpdate: (id: string) => void;
  onUninstall: (p: InstalledPluginView) => void;
  onBrowse: () => void;
}) {
  const list = data.installed.filter((p) =>
    matchesQuery(query, p.name, p.display_name, p.description, p.marketplace),
  );

  if (data.installed.length === 0) {
    return (
      <EmptyState
        icon={<Puzzle />}
        title="No plugins installed"
        body="Plugins add slash commands, agents, skills and MCP servers."
        action={
          <button
            type="button"
            onClick={onBrowse}
            className="text-[13px] font-medium text-mira-purple hover:underline"
          >
            Browse plugins
          </button>
        }
      />
    );
  }

  return (
    <>
      {list.length === 0 ? (
        <div className="py-12 text-center text-[13px] text-muted-foreground">No installed plugins match.</div>
      ) : (
        <div className="grid gap-2.5 md:grid-cols-2">
          {list.map((p) => {
            const catalogEntry = data.catalog.find((c) => c.id === p.id);
            const iconSrc = catalogEntry?.icon_url ?? faviconSrc(catalogEntry?.homepage ?? null);
            return (
              <InstalledCard
                key={p.id}
                plugin={p}
                iconSrc={iconSrc ?? null}
                busy={busy}
                onOpen={onOpen}
                onToggle={onToggle}
                onUpdate={onUpdate}
                onUninstall={onUninstall}
              />
            );
          })}
        </div>
      )}
    </>
  );
}

function AuthorBadge({ name }: { name: string }) {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) % 360;
  const letter = name.match(/[a-z0-9]/i)?.[0]?.toUpperCase() ?? '?';
  return (
    <span
      className="inline-flex size-4 shrink-0 items-center justify-center rounded-full text-[9px] font-bold"
      style={{ background: `hsl(${h} 45% 22%)`, color: `hsl(${h} 80% 72%)` }}
    >
      {letter}
    </span>
  );
}

function InstalledCard({
  plugin: p,
  iconSrc,
  busy,
  onOpen,
  onToggle,
  onUpdate,
  onUninstall,
}: {
  plugin: InstalledPluginView;
  iconSrc: string | null;
  busy: string | null;
  onOpen: (id: string) => void;
  onToggle: (id: string, enabled: boolean) => void;
  onUpdate: (id: string) => void;
  onUninstall: (p: InstalledPluginView) => void;
}) {
  const title = ((p.display_name || p.name) || '').replace(/^\w/, (c) => c.toUpperCase());
  const author = p.author || p.marketplace;

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => onOpen(p.id)}
      onKeyDown={(e) => (e.key === 'Enter' ? onOpen(p.id) : undefined)}
      className="group flex cursor-pointer gap-3.5 rounded-2xl border border-border/50 bg-white/[0.035] p-4 text-left transition-all hover:border-border/70 hover:bg-white/[0.055]"
    >
      <Avatar name={p.id} src={iconSrc} size="md" />

      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        {/* Title row */}
        <div className="flex items-start justify-between gap-2">
          <div className="min-w-0 flex-1">
            <div className="truncate text-[13.5px] font-semibold leading-tight">{title}</div>
            {author && (
              <div className="mt-0.5 flex items-center gap-1">
                <AuthorBadge name={author} />
                <span className="truncate text-[11px] text-muted-foreground/70">by {author}</span>
              </div>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-1" onClick={(e) => e.stopPropagation()}>
            <span className="hidden text-[11px] text-muted-foreground/50 sm:inline">{relativeTime(p.updated_at)}</span>
            <Switch
              checked={p.enabled}
              disabled={!!busy}
              label={p.enabled ? `Disable ${p.name}` : `Enable ${p.name}`}
              onChange={(v) => onToggle(p.id, v)}
            />
            <RowMenu
              items={[
                { label: 'Details',   icon: <Info />,     onSelect: () => onOpen(p.id) },
                { label: 'Update',    icon: <RotateCcw />, onSelect: () => onUpdate(p.id), disabled: !!busy },
                { label: 'Uninstall', icon: <Trash2 />,   danger: true, onSelect: () => onUninstall(p) },
              ]}
            />
          </div>
        </div>

        {/* Description */}
        {p.description && (
          <p className="line-clamp-2 text-[12px] leading-relaxed text-muted-foreground/80">{p.description}</p>
        )}

        {/* Component pills */}
        {p.commands.length + p.agents.length + p.skills.length + p.mcp_servers.length > 0 && (
          <div className="flex flex-wrap gap-1">
            {p.commands.length > 0 && <Pill tone="blue">{plural(p.commands.length, 'command')}</Pill>}
            {p.agents.length > 0 && <Pill tone="purple">{plural(p.agents.length, 'agent')}</Pill>}
            {p.skills.length > 0 && <Pill tone="purple">{plural(p.skills.length, 'skill')}</Pill>}
            {p.mcp_servers.length > 0 && <Pill tone="green">{plural(p.mcp_servers.length, 'MCP server')}</Pill>}
            {p.hooks.length > 0 && (
              <Pill title="Runs at points in each turn (before and after tools, on submit, on stop)">{plural(p.hooks.length, 'hook')}</Pill>
            )}
          </div>
        )}

        {p.problems.map((pr) => (
          <div key={pr} className="text-[12px] text-amber-300">{pr}</div>
        ))}

        {!p.enabled && (
          <span className="self-start rounded-full border border-border/40 px-2 py-0.5 text-[10.5px] text-muted-foreground/60">
            Disabled
          </span>
        )}
      </div>
    </div>
  );
}
