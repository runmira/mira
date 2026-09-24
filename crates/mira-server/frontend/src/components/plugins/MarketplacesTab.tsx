import { useState } from 'react';
import { AlertCircle, Folder, GitBranch, Globe, Plus, RotateCcw, Trash2 } from 'lucide-react';
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
          <Plus className="size-3.5" />
          {adding ? 'Adding…' : 'Add marketplace'}
        </Button>
      </form>

      {data.marketplaces.length > 0 && (
        <ul className="flex flex-col divide-y divide-border/50 overflow-hidden rounded-2xl border border-border/60 bg-white/[0.02]">
          {data.marketplaces.map((m) => (
            <li key={m.name} className="flex items-center gap-3.5 px-4 py-4">
              <Avatar name={m.name} />
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-[13.5px] font-semibold">{m.name}</span>
                  <span className="text-[11.5px] text-muted-foreground/70">
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
                  <p className="mt-1 line-clamp-2 text-[12px] text-muted-foreground">{m.description}</p>
                )}
                {m.error && (
                  <div className="mt-1 flex items-center gap-1 text-[12px] text-destructive">
                    <AlertCircle className="size-3.5" />
                    {m.error}
                  </div>
                )}
              </div>
              <Button
                variant="outline"
                size="sm"
                className="h-7 gap-1.5 rounded-lg"
                disabled={!!busy}
                onClick={() => onUpdate(m.name)}
              >
                <RotateCcw className="size-3.5" />
                {busy === `market-update:${m.name}` ? 'Updating…' : 'Update'}
              </Button>
              <RowMenu
                items={[
                  { label: 'Remove', icon: <Trash2 />, danger: true, onSelect: () => onRemove(m) },
                ]}
              />
            </li>
          ))}
        </ul>
      )}

      {suggestions.length > 0 && (
        <div className="flex flex-col gap-2">
          <div className="text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/60">
            Suggested
          </div>
          {suggestions.map((s) => (
            <div
              key={s.source}
              className="flex items-center gap-3.5 rounded-xl border border-border/50 bg-white/[0.02] px-4 py-3"
            >
              <Avatar name={s.name} size="sm" />
              <div className="min-w-0 flex-1">
                <div className="font-mono text-[12.5px]">{s.source}</div>
                <div className="text-[12px] text-muted-foreground">{s.description}</div>
              </div>
              <Button
                size="sm"
                variant="outline"
                className="h-7 rounded-lg"
                disabled={!!busy}
                onClick={() => submit(s.source)}
              >
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
  if (kind === 'directory') return <Folder className={cls} />;
  if (kind === 'url') return <Globe className={cls} />;
  return <GitBranch className={cls} />;
}
