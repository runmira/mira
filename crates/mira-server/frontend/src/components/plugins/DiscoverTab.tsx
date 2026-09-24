import { useMemo, useState } from 'react';
import { CheckCircle2, Store } from 'lucide-react';
import type { CatalogEntry, PluginsOverview } from '../../api';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { Avatar, EmptyState, Pill } from './shared';

export function matchesQuery(q: string, ...fields: (string | null | undefined)[]): boolean {
  if (!q) return true;
  const needle = q.toLowerCase();
  return fields.some((f) => f?.toLowerCase().includes(needle));
}

export function DiscoverTab({
  data,
  query,
  busy,
  onOpen,
  onInstall,
  onAddSuggested,
}: {
  data: PluginsOverview;
  query: string;
  busy: string | null;
  onOpen: (id: string) => void;
  onInstall: (id: string) => void;
  onAddSuggested: (source: string) => void;
}) {
  const [category, setCategory] = useState<string>('all');
  const [market, setMarket] = useState<string>('all');

  const categories = useMemo(() => {
    const counts = new Map<string, number>();
    for (const c of data.catalog) {
      const k = c.category ?? 'other';
      counts.set(k, (counts.get(k) ?? 0) + 1);
    }
    return [...counts.entries()].sort((a, b) => b[1] - a[1]).map(([k]) => k);
  }, [data.catalog]);

  const shown = data.catalog.filter(
    (c) =>
      (category === 'all' || (c.category ?? 'other') === category) &&
      (market === 'all' || c.marketplace === market) &&
      matchesQuery(query, c.name, c.display_name, c.description, c.author, c.category, ...c.keywords, ...c.tags),
  );

  if (data.marketplaces.length === 0) {
    return (
      <EmptyState
        icon={<Store />}
        title="Add a marketplace to browse plugins"
        body={
          <>
            Marketplaces are catalogs of plugins. Mira reads Claude Code's format, so the same
            plugins work in both.
          </>
        }
        action={
          <div className="mt-2 flex w-full max-w-md flex-col gap-2">
            {data.suggested_marketplaces.map((s) => (
              <button
                key={s.source}
                type="button"
                disabled={!!busy}
                onClick={() => onAddSuggested(s.source)}
                className="flex items-center gap-3 rounded-xl border border-border/60 bg-white/[0.03] px-4 py-3 text-left transition-all hover:border-border hover:bg-white/[0.06] disabled:opacity-60"
              >
                <Avatar name={s.name} size="sm" />
                <div className="min-w-0 flex-1">
                  <div className="text-[13px] font-medium">{s.source}</div>
                  <div className="truncate text-[12px] text-muted-foreground">{s.description}</div>
                </div>
                <span className="shrink-0 text-[12px] font-medium text-mira-purple">
                  {busy === `market-add:${s.source}` ? 'Adding…' : 'Add'}
                </span>
              </button>
            ))}
          </div>
        }
      />
    );
  }

  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-wrap items-center gap-1.5">
        <Chip active={category === 'all'} onClick={() => setCategory('all')}>
          All
        </Chip>
        {categories.map((c) => (
          <Chip key={c} active={category === c} onClick={() => setCategory(c)}>
            {c}
          </Chip>
        ))}
        {data.marketplaces.length > 1 && (
          <select
            value={market}
            onChange={(e) => setMarket(e.target.value)}
            className="ml-auto h-7 rounded-lg border border-border/60 bg-transparent px-2 text-[12px] text-muted-foreground"
            aria-label="Marketplace"
          >
            <option value="all">All marketplaces</option>
            {data.marketplaces.map((m) => (
              <option key={m.name} value={m.name}>
                {m.name}
              </option>
            ))}
          </select>
        )}
      </div>

      {shown.length === 0 ? (
        <div className="py-12 text-center text-[13px] text-muted-foreground">No plugins match.</div>
      ) : (
        <div className="grid gap-3 md:grid-cols-2">
          {shown.map((c) => (
            <PluginCard
              key={c.id}
              entry={c}
              busy={busy}
              showMarket={data.marketplaces.length > 1}
              onOpen={() => onOpen(c.id)}
              onInstall={() => onInstall(c.id)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function Chip({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'h-7 rounded-full border px-3.5 text-[12px] capitalize transition-all',
        active
          ? 'border-mira-purple/40 bg-mira-purple/10 text-mira-purple'
          : 'border-border/60 text-muted-foreground hover:border-border hover:text-foreground',
      )}
    >
      {children}
    </button>
  );
}

function PluginCard({
  entry,
  busy,
  showMarket,
  onOpen,
  onInstall,
}: {
  entry: CatalogEntry;
  busy: string | null;
  showMarket: boolean;
  onOpen: () => void;
  onInstall: () => void;
}) {
  const installing = busy === `install:${entry.id}`;
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onOpen}
      onKeyDown={(e) => (e.key === 'Enter' ? onOpen() : undefined)}
      className="group relative flex cursor-pointer gap-3.5 rounded-2xl border border-border/50 bg-white/[0.02] p-4 text-left transition-all hover:border-border/80 hover:bg-white/[0.04] hover:shadow-sm"
    >
      <Avatar name={entry.id} src={entry.icon_url} />
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex items-start gap-2">
          <div className="min-w-0 flex-1">
            <div className="truncate text-[13.5px] font-semibold leading-snug">
              {entry.display_name || entry.name}
            </div>
            <div className="mt-0.5 truncate text-[11.5px] text-muted-foreground/80">
              {[entry.author, showMarket ? entry.marketplace : null].filter(Boolean).join(' · ') || entry.marketplace}
            </div>
          </div>
          {entry.installed ? (
            <Pill tone={entry.enabled ? 'green' : 'neutral'} className="mt-0.5 shrink-0">
              {entry.enabled ? (
                <>
                  <CheckCircle2 className="size-3" /> Installed
                </>
              ) : (
                'Disabled'
              )}
            </Pill>
          ) : (
            <Button
              size="sm"
              variant="outline"
              className="h-7 shrink-0 rounded-lg px-3 text-[12px]"
              disabled={!entry.installable || !!busy}
              title={!entry.installable ? "Mira can't install this source type yet" : undefined}
              onClick={(e) => {
                e.stopPropagation();
                onInstall();
              }}
            >
              {installing ? 'Installing…' : 'Install'}
            </Button>
          )}
        </div>
        <p className="mt-2 line-clamp-2 text-[12px] leading-relaxed text-muted-foreground">
          {entry.description || 'No description.'}
        </p>
      </div>
    </div>
  );
}
