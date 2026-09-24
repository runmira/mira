import { Info, Puzzle, RotateCcw, Trash2 } from 'lucide-react';
import type { InstalledPluginView, PluginsOverview } from '../../api';
import { matchesQuery } from './DiscoverTab';
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
          <button type="button" onClick={onBrowse} className="text-[13px] font-medium text-mira-purple hover:underline">
            Browse plugins
          </button>
        }
      />
    );
  }
  return (
    <ul className="flex flex-col divide-y divide-border/50 overflow-hidden rounded-2xl border border-border/60 bg-white/[0.02]">
      {list.map((p) => (
        <li
          key={p.id}
          className="flex cursor-pointer items-start gap-3.5 px-4 py-4 transition-colors hover:bg-white/[0.03]"
          onClick={() => onOpen(p.id)}
        >
          <Avatar name={p.id} />
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-[13.5px] font-semibold">{p.display_name || p.name}</span>
              <span className="text-[11.5px] text-muted-foreground/70">
                {p.marketplace} · {p.version.length > 12 ? p.version.slice(0, 7) : p.version}
              </span>
              {!p.enabled && <Pill>Disabled</Pill>}
            </div>
            {p.description && (
              <p className="mt-0.5 line-clamp-1 text-[12px] text-muted-foreground">{p.description}</p>
            )}
            <div className="mt-2 flex flex-wrap gap-1.5">
              {p.commands.length > 0 && <Pill tone="blue">{plural(p.commands.length, 'command')}</Pill>}
              {p.agents.length > 0 && <Pill tone="purple">{plural(p.agents.length, 'agent')}</Pill>}
              {p.skills.length > 0 && <Pill tone="purple">{plural(p.skills.length, 'skill')}</Pill>}
              {p.mcp_servers.length > 0 && <Pill tone="green">{plural(p.mcp_servers.length, 'MCP server')}</Pill>}
              {p.hooks.length > 0 && (
                <Pill title="Mira doesn't run plugin hooks yet">{plural(p.hooks.length, 'hook')} · not run</Pill>
              )}
              {p.lsp_servers.length > 0 && (
                <Pill title="Mira doesn't run LSP servers yet">LSP · not run</Pill>
              )}
            </div>
            {p.problems.map((pr) => (
              <div key={pr} className="mt-1.5 text-[12px] text-amber-300">
                {pr}
              </div>
            ))}
          </div>
          <div className="flex shrink-0 items-center gap-1.5" onClick={(e) => e.stopPropagation()}>
            <span className="hidden text-[11px] text-muted-foreground/60 sm:inline">{relativeTime(p.updated_at)}</span>
            <Switch
              checked={p.enabled}
              disabled={!!busy}
              label={p.enabled ? `Disable ${p.name}` : `Enable ${p.name}`}
              onChange={(v) => onToggle(p.id, v)}
            />
            <RowMenu
              items={[
                { label: 'Details', icon: <Info />, onSelect: () => onOpen(p.id) },
                { label: 'Update', icon: <RotateCcw />, onSelect: () => onUpdate(p.id), disabled: !!busy },
                { label: 'Uninstall', icon: <Trash2 />, danger: true, onSelect: () => onUninstall(p) },
              ]}
            />
          </div>
        </li>
      ))}
      {list.length === 0 && (
        <li className="px-4 py-10 text-center text-[13px] text-muted-foreground">No installed plugins match.</li>
      )}
    </ul>
  );
}
