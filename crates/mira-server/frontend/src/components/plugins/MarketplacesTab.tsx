import { useState } from 'react';
import { ArrowClockwise, GitBranch, Globe, FolderSimple, Plus, Trash, WarningCircle } from '@phosphor-icons/react';
import type { MarketplaceView, PluginsOverview } from '../../api';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Avatar, RowMenu, plural, relativeTime } from './shared';

export function MarketplacesTab({
  data,
  busy,
  onAdd,
  onUpdate,
  onRemove,
}: {
  data: PluginsOverview;
  busy: string | null;
  onAdd: (source: string) => Promise<boolean>;
  onUpdate: (name: string) => void;
  onRemove: (m: MarketplaceView) => void;
}) {
  const [source, setSource] = useState('');
  const adding = busy?.startsWith('market-add:');
  const added = new Set(data.marketplaces.map((m) => m.source.split('#')[0]));
  const suggestions = data.suggested_marketplaces.filter((s) => !added.has(s.source));

  async function submit(src: string) {
    if (!src.trim()) return;
    if (await onAdd(src.trim())) setSource('');
  }

  return (
    <div className="flex flex-col gap-5">
      <form
        className="flex gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          submit(source);
        }}
      >
        <Input
          value={source}
          onChange={(e) => setSource(e.target.value)}
          placeholder="owner/repo, git URL, URL to marketplace.json, or local path"
          className="h-9 flex-1 font-mono text-[12.5px]"
          disabled={adding}
        />
        <Button type="submit" size="sm" className="h-9 gap-1.5" disabled={!source.trim() || !!busy}>
          <Plus className="size-3.5" weight="bold" />
          {adding ? 'Adding…' : 'Add marketplace'}
        </Button>
      </form>

      {data.marketplaces.length > 0 && (
        <ul className="flex flex-col divide-y divide-border/60 overflow-hidden rounded-xl border border-border/70 bg-card/40">
          {data.marketplaces.map((m) => (
            <li key={m.name} className="flex items-center gap-3 px-4 py-3.5">
              <Avatar name={m.name} />
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-[14px] font-medium">{m.name}</span>
                  <span className="text-[11.5px] text-muted-foreground">
                    {plural(m.plugin_count, 'plugin')}
                    {m.owner ? ` · ${m.owner}` : ''}
                  </span>
                </div>
                <div className="mt-0.5 flex items-center gap-1.5 text-[12px] text-muted-foreground">
                  <SourceIcon kind={m.kind} />
                  <span className="truncate font-mono text-[11.5px]">{m.source}</span>
                  {m.updated_at > 0 && <span className="shrink-0">· updated {relativeTime(m.updated_at)}</span>}
                </div>
                {m.description && (
                  <p className="mt-1 line-clamp-2 text-[12.5px] text-muted-foreground">{m.description}</p>
                )}
                {m.error && (
                  <div className="mt-1 flex items-center gap-1 text-[12px] text-destructive">
                    <WarningCircle className="size-3.5" weight="fill" />
                    {m.error}
                  </div>
                )}
              </div>
              <Button
                variant="outline"
                size="sm"
                className="h-7 gap-1.5"
                disabled={!!busy}
                onClick={() => onUpdate(m.name)}
              >
                <ArrowClockwise className="size-3.5" />
                {busy === `market-update:${m.name}` ? 'Updating…' : 'Update'}
              </Button>
              <RowMenu
                items={[
                  { label: 'Remove', icon: <Trash />, danger: true, onSelect: () => onRemove(m) },
                ]}
              />
            </li>
          ))}
        </ul>
      )}

      {suggestions.length > 0 && (
        <div className="flex flex-col gap-2">
          <div className="text-[12px] font-medium uppercase tracking-wider text-muted-foreground">Suggested</div>
          {suggestions.map((s) => (
            <div key={s.source} className="flex items-center gap-3 rounded-xl border border-border/70 bg-card/40 px-4 py-3">
              <Avatar name={s.name} size="sm" />
              <div className="min-w-0 flex-1">
                <div className="font-mono text-[12.5px]">{s.source}</div>
                <div className="text-[12px] text-muted-foreground">{s.description}</div>
              </div>
              <Button size="sm" variant="outline" className="h-7" disabled={!!busy} onClick={() => submit(s.source)}>
                {busy === `market-add:${s.source}` ? 'Adding…' : 'Add'}
              </Button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function SourceIcon({ kind }: { kind: MarketplaceView['kind'] }) {
  const cls = 'size-3.5 shrink-0';
  if (kind === 'directory') return <FolderSimple className={cls} />;
  if (kind === 'url') return <Globe className={cls} />;
  return <GitBranch className={cls} />;
}
