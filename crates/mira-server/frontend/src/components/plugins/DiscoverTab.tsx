import { useMemo, useState } from 'react';
import { CheckCircle2, Store } from 'lucide-react';
import type { CatalogEntry, PluginsOverview } from '../../api';
import { cn } from '@/lib/utils';
import { Avatar, EmptyState, Pill } from './shared';

export function matchesQuery(q: string, ...fields: (string | null | undefined)[]): boolean {
  if (!q) return true;
  const needle = q.toLowerCase();
  return fields.some((f) => f?.toLowerCase().includes(needle));
}

/** Generic hosting domains whose favicon isn't the plugin's brand logo. */
const GENERIC_HOSTS = new Set([
  'github.com', 'gitlab.com', 'bitbucket.org',
  'npmjs.com', 'pypi.org', 'crates.io',
]);

/** Derive a favicon URL from a homepage, or null if the domain is too generic.
 *  Always uses the apex domain (e.g. mcp.figma.com → figma.com) so Google's
 *  favicon service returns the brand logo instead of a generic globe. */
export function faviconSrc(homepage: string | null | undefined): string | null {
  if (!homepage) return null;
  try {
    const { hostname } = new URL(homepage);
    const parts = hostname.split('.');
    const apex = parts.length > 2 ? parts.slice(-2).join('.') : hostname;
    if (GENERIC_HOSTS.has(apex)) return null;
    return `https://www.google.com/s2/favicons?domain=${apex}&sz=64`;
  } catch {
    return null;
  }
}

/** Returns the resolved icon src for a catalog entry. */
function iconSrc(entry: CatalogEntry): string | null {
  return entry.icon_url ?? faviconSrc(entry.homepage);
}

/** Sort so entries with a real favicon/icon come first, then alphabetical. */
function sortByIcon(entries: CatalogEntry[]): CatalogEntry[] {
  return [...entries].sort((a, b) => {
    const aHas = iconSrc(a) ? 1 : 0;
    const bHas = iconSrc(b) ? 1 : 0;
    if (bHas !== aHas) return bHas - aHas;
    return (a.display_name || a.name).localeCompare(b.display_name || b.name);
  });
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

  // Categories sorted by count descending
  const categories = useMemo(() => {
    const counts = new Map<string, number>();
    for (const c of data.catalog) {
      const k = c.category ?? 'other';
      counts.set(k, (counts.get(k) ?? 0) + 1);
    }
    return [...counts.entries()].sort((a, b) => b[1] - a[1]).map(([k]) => k);
  }, [data.catalog]);

  const filtered = data.catalog.filter(
    (c) =>
      (category === 'all' || (c.category ?? 'other') === category) &&
      (market === 'all' || c.marketplace === market) &&
      matchesQuery(query, c.name, c.display_name, c.description, c.author, c.category, ...c.keywords, ...c.tags),
  );

  // Favicon-first, then alphabetical
  const shown = sortByIcon(filtered);

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
      {/* Category chips */}
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
        <div className="grid gap-2.5 md:grid-cols-2">
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
        'inline-flex h-7 items-center gap-1.5 rounded-lg px-3 text-[12px] capitalize transition-all',
        active
          ? 'bg-mira-purple/18 font-semibold text-mira-purple ring-2 ring-white/20'
          : 'bg-white/[0.05] text-muted-foreground/65 hover:bg-white/[0.09] hover:text-foreground',
      )}
    >
      {active && <span className="size-1.5 shrink-0 rounded-full bg-mira-purple" />}
      {children}
    </button>
  );
}

/** Small coloured initial badge for the author. */
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
  const src = iconSrc(entry);
  const author = entry.author || (showMarket ? entry.marketplace : null);

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onOpen}
      onKeyDown={(e) => (e.key === 'Enter' ? onOpen() : undefined)}
      className="group flex cursor-pointer gap-3.5 rounded-2xl border border-border/60 bg-white/[0.08] p-4 text-left shadow-sm shadow-black/30 transition-all hover:border-border/80 hover:bg-white/[0.11] hover:shadow-md hover:shadow-black/40"
    >
      {/* Icon */}
      <Avatar name={entry.id} src={src} size="md" />

      {/* Body */}
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        {/* Title row */}
        <div className="flex items-start justify-between gap-2">
          <div className="min-w-0 flex-1">
            <div className="truncate text-[13.5px] font-semibold leading-tight">
              {((entry.display_name || entry.name) || '').replace(/^\w/, (c) => c.toUpperCase())}
            </div>
            {author && (
              <div className="mt-0.5 flex items-center gap-1">
                <AuthorBadge name={author} />
                <span className="truncate text-[11px] text-muted-foreground/70">
                  by {author}
                </span>
              </div>
            )}
          </div>

          {/* Action */}
          {entry.installed ? (
            <Pill tone={entry.enabled ? 'green' : 'neutral'} className="mt-0.5 shrink-0">
              {entry.enabled ? (
                <><CheckCircle2 className="size-3" /> Installed</>
              ) : (
                'Disabled'
              )}
            </Pill>
          ) : (
            <button
              type="button"
              disabled={!entry.installable || !!busy}
              title={!entry.installable ? "Mira can't install this source type yet" : undefined}
              onClick={(e) => { e.stopPropagation(); onInstall(); }}
              className={cn(
                'shrink-0 rounded-full bg-white px-3 py-1 text-[12px] font-medium text-black transition-all',
                'disabled:opacity-40',
                !entry.installable || !!busy ? 'cursor-not-allowed' : 'hover:bg-white/90 active:scale-95',
              )}
            >
              {installing ? 'Installing…' : '+ Install'}
            </button>
          )}
        </div>

        {/* Description */}
        <p className="line-clamp-2 text-[12px] leading-relaxed text-muted-foreground/80">
          {entry.description || 'No description.'}
        </p>

        {/* Category tag */}
        {entry.category && (
          <span className="self-start rounded-md bg-white/[0.07] px-2 py-0.5 text-[10.5px] capitalize text-muted-foreground/60">
            {entry.category}
          </span>
        )}
      </div>
    </div>
  );
}
