import { useState } from 'react';
import { AlertCircle, Folder, GitBranch, Globe, Plus, RotateCcw, Store, Trash2 } from 'lucide-react';
import type { MarketplaceView, PluginsOverview } from '../../api';
import { Button } from '@/components/ui/button';
import { Avatar, EmptyState, RowMenu, plural, relativeTime } from './shared';

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
    <div className="flex flex-col gap-6">
      {/* Add form */}
      <form
        className="flex gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          submit(source);
        }}
      >
        <input
          value={source}
          onChange={(e) => setSource(e.target.value)}
          placeholder="owner/repo, git URL, marketplace.json URL, or local path"
          disabled={!!adding}
          className="h-9 min-w-0 flex-1 rounded-xl border border-border/60 bg-white/[0.03] px-3 font-mono text-[12.5px] outline-none placeholder:text-muted-foreground/40 focus:border-border focus:bg-white/[0.05] disabled:opacity-60"
        />
        <Button
          type="submit"
          size="sm"
          className="h-9 shrink-0 gap-1.5"
          disabled={!source.trim() || !!busy}
        >
          <Plus className="size-3.5" />
          {adding ? 'Adding…' : 'Add'}
        </Button>
      </form>

      {/* Added marketplaces */}
      {data.marketplaces.length > 0 && (
        <section>
          <h3 className="mb-2.5 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/60">
            Added
          </h3>
          <ul className="flex flex-col divide-y divide-border/50 overflow-hidden rounded-2xl border border-border/60 bg-white/[0.035]">
            {data.marketplaces.map((m) => (
              <MarketplaceRow
                key={m.name}
                market={m}
                busy={busy}
                onUpdate={onUpdate}
                onRemove={onRemove}
              />
            ))}
          </ul>
        </section>
      )}

      {/* Suggestions */}
      {suggestions.length > 0 && (
        <section>
          <h3 className="mb-2.5 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/60">
            Suggested
          </h3>
          <div className="flex flex-col gap-2">
            {suggestions.map((s) => (
              <button
                key={s.source}
                type="button"
                disabled={!!busy}
                onClick={() => submit(s.source)}
                className="flex items-center gap-3.5 rounded-xl border border-border/50 bg-white/[0.035] px-4 py-3.5 text-left transition-all hover:border-border/70 hover:bg-white/[0.055] disabled:opacity-60"
              >
                <Avatar name={s.name} size="sm" />
                <div className="min-w-0 flex-1">
                  <div className="font-mono text-[12.5px] font-medium">{s.source}</div>
                  <div className="mt-0.5 text-[12px] text-muted-foreground">{s.description}</div>
                </div>
                <span className="shrink-0 text-[12px] font-medium text-mira-purple">
                  {busy === `market-add:${s.source}` ? 'Adding…' : '+ Add'}
                </span>
              </button>
            ))}
          </div>
        </section>
      )}

      {data.marketplaces.length === 0 && suggestions.length === 0 && (
        <EmptyState
          icon={<Store />}
          title="No marketplaces yet"
          body="Marketplaces catalog plugins you can install. Paste a GitHub repo or URL above."
        />
      )}
    </div>
  );
}

function MarketplaceRow({
  market: m,
  busy,
  onUpdate,
  onRemove,
}: {
  market: MarketplaceView;
  busy: string | null;
  onUpdate: (name: string) => void;
  onRemove: (m: MarketplaceView) => void;
}) {
  return (
    <li className="flex items-center gap-3.5 px-4 py-4">
      <Avatar name={m.name} />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-[13.5px] font-semibold">{m.name}</span>
          <span className="rounded-full border border-border/40 px-2 py-0.5 text-[10.5px] text-muted-foreground/60">
            {plural(m.plugin_count, 'plugin')}
          </span>
        </div>
        <div className="mt-0.5 flex items-center gap-1.5 text-muted-foreground">
          <SourceIcon kind={m.kind} />
          <span className="truncate font-mono text-[11.5px]">{m.source}</span>
          {m.updated_at > 0 && (
            <span className="shrink-0 text-[11px] text-muted-foreground/60">· {relativeTime(m.updated_at)}</span>
          )}
        </div>
        {m.description && (
          <p className="mt-0.5 line-clamp-1 text-[12px] text-muted-foreground/70">{m.description}</p>
        )}
        {m.error && (
          <div className="mt-1 flex items-center gap-1 text-[12px] text-destructive">
            <AlertCircle className="size-3.5 shrink-0" />
            {m.error}
          </div>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-1.5">
        <Button
          variant="outline"
          size="sm"
          className="h-7 gap-1.5 rounded-lg"
          disabled={!!busy}
          onClick={() => onUpdate(m.name)}
        >
          <RotateCcw className="size-3.5" />
          {busy === `market-update:${m.name}` ? 'Syncing…' : 'Sync'}
        </Button>
        <RowMenu
          items={[
            { label: 'Remove', icon: <Trash2 />, danger: true, onSelect: () => onRemove(m) },
          ]}
        />
      </div>
    </li>
  );
}

function SourceIcon({ kind }: { kind: MarketplaceView['kind'] }) {
  const cls = 'size-3.5 shrink-0';
  if (kind === 'directory') return <Folder className={cls} />;
  if (kind === 'url') return <Globe className={cls} />;
  return <GitBranch className={cls} />;
}
