import { useEffect, useMemo, useRef, useState } from 'react';
import { Check, FolderPlus, Search } from 'lucide-react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { basename, cn } from '@/lib/utils';
import { useResource } from '../state/resources/hooks';
import { SidebarFolderIcon } from './sidebar/navigation';
import { timeAgo } from './sidebar/sessionRows';

/**
 * The project switcher: every folder Mira has a chat in, one keystroke
 * away, plus a way to add a new one.
 *
 * Where the *folder* picker answers "where is it on disk", this answers
 * "which one do I mean" — so the list is built from the chats that
 * already exist, most recent first, with the current project checked.
 *
 * It opens as a popover on whatever opened it (the empty state's title,
 * the composer's project chip), so the choice stays next to the thing
 * being changed. Rows look like the sidebar's folders: the same themed
 * icon, with how many chats live there and when you were last in it.
 *
 * Switching starts a **new** chat in that folder. The server
 * materialises a fresh slot per cwd.
 */
export function ProjectSwitcher({
  cwd,
  onPicked,
  onNewProject,
  children,
  side = 'bottom',
  align = 'center',
}: {
  /** The chat's current folder; shown checked. */
  cwd: string;
  /** Fires with the chosen folder. The caller swaps the cwd, which the
   *  server turns into a new session. */
  onPicked: (path: string) => void;
  /** "Open folder…" — hands off to the platform folder panel. */
  onNewProject: () => void;
  /** The control that opens it. */
  children: React.ReactNode;
  side?: 'top' | 'bottom';
  align?: 'start' | 'center' | 'end';
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [highlight, setHighlight] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);
  const sessions = useResource('sessions', open);

  // One entry per folder, most recently touched first, with what's in it.
  const folders = useMemo(() => {
    const byCwd = new Map<string, { path: string; chats: number; last: number }>();
    for (const s of sessions.data ?? []) {
      if (!s.cwd) continue;
      const entry = byCwd.get(s.cwd) ?? { path: s.cwd, chats: 0, last: 0 };
      entry.chats += 1;
      entry.last = Math.max(entry.last, s.updated_at ?? 0);
      byCwd.set(s.cwd, entry);
    }
    if (cwd && !byCwd.has(cwd)) byCwd.set(cwd, { path: cwd, chats: 0, last: 0 });
    return [...byCwd.values()].sort(
      (a, b) => Number(b.path === cwd) - Number(a.path === cwd) || b.last - a.last,
    );
  }, [sessions.data, cwd]);

  const projects = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (q ? folders.filter((f) => f.path.toLowerCase().includes(q)) : folders).slice(0, 40);
  }, [folders, query]);

  useEffect(() => {
    if (!open) return;
    setQuery('');
    setHighlight(0);
  }, [open]);

  useEffect(() => setHighlight(0), [query]);

  useEffect(() => {
    listRef.current
      ?.querySelector<HTMLElement>(`[data-index="${highlight}"]`)
      ?.scrollIntoView({ block: 'nearest' });
  }, [highlight]);

  const pick = (path: string) => {
    setOpen(false);
    if (path !== cwd) onPicked(path);
  };

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>{children}</PopoverTrigger>
      <PopoverContent
        side={side}
        align={align}
        sideOffset={8}
        onOpenAutoFocus={(e) => {
          e.preventDefault();
          inputRef.current?.focus();
        }}
        className="w-[22rem] max-w-[calc(100vw-2rem)] overflow-hidden rounded-xl border-border/60 bg-popover/95 p-0 backdrop-blur"
      >
        {/* Search */}
        <div className="p-1.5">
          <label className="flex items-center gap-2 rounded-lg bg-fg/[0.04] px-2.5 py-1.5 focus-within:ring-2 focus-within:ring-mira-blue/15">
            <Search className="size-3.5 shrink-0 text-muted-foreground/70" aria-hidden />
            <input
              ref={inputRef}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                  e.preventDefault();
                  const step = e.key === 'ArrowDown' ? 1 : -1;
                  setHighlight((h) => (projects.length ? (h + step + projects.length) % projects.length : 0));
                }
                // Enter opens the highlighted match: the list never
                // needs the mouse.
                if (e.key === 'Enter' && projects[highlight]) {
                  e.preventDefault();
                  pick(projects[highlight].path);
                }
              }}
              placeholder="Switch project…"
              aria-label="Search projects"
              spellCheck={false}
              className="min-w-0 flex-1 bg-transparent text-[13px] text-foreground outline-none placeholder:text-muted-foreground/60"
            />
          </label>
        </div>

        <div className="px-3 pb-1 pt-0.5 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground/60">
          {query.trim() ? 'Matches' : 'Recent projects'}
        </div>

        <div ref={listRef} role="listbox" aria-label="Projects" className="max-h-[min(20rem,50vh)] overflow-y-auto px-1.5 pb-1.5">
          {projects.length === 0 && (
            <div className="px-2 py-3 text-[12.5px] text-muted-foreground">
              {sessions.isLoading ? 'Loading projects…' : query.trim() ? `No project matches “${query.trim()}”` : 'No projects yet'}
            </div>
          )}
          {projects.map((f, i) => {
            const active = f.path === cwd;
            const name = basename(f.path) || f.path;
            const when = timeAgo(f.last);
            return (
              <button
                key={f.path}
                type="button"
                role="option"
                aria-selected={active}
                data-index={i}
                onClick={() => pick(f.path)}
                onPointerMove={() => setHighlight(i)}
                title={f.path}
                className={cn(
                  'flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left transition-colors',
                  i === highlight ? 'bg-fg/[0.07]' : 'hover:bg-fg/[0.05]',
                )}
              >
                <SidebarFolderIcon name={name} open={active} current={active} className="size-4" />
                <span className="flex min-w-0 flex-1 flex-col">
                  <span
                    className={cn(
                      'truncate text-[13.5px] leading-5',
                      active ? 'font-semibold text-foreground' : 'font-medium text-foreground/90',
                    )}
                  >
                    {name}
                  </span>
                  {/* Where it lives: two projects can share a name. */}
                  <span className="truncate text-[11px] leading-4 text-muted-foreground/60">
                    {shortParent(f.path)}
                  </span>
                </span>
                <span className="flex shrink-0 flex-col items-end text-[11px] leading-4 tabular-nums text-muted-foreground/60">
                  {f.chats > 0 && (
                    <span>
                      {f.chats} chat{f.chats === 1 ? '' : 's'}
                    </span>
                  )}
                  {when && <span className="text-muted-foreground/45">{when}</span>}
                </span>
                <span className="flex size-4 shrink-0 items-center justify-center">
                  {active && <Check className="size-3.5 text-mira-blue" aria-label="Current project" />}
                </span>
              </button>
            );
          })}
        </div>

        {/* A folder Mira hasn't seen yet */}
        <div className="border-t border-border/50 p-1.5">
          <button
            type="button"
            onClick={() => {
              setOpen(false);
              onNewProject();
            }}
            className="flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left text-[13px] text-foreground/85 transition-colors hover:bg-fg/[0.05] hover:text-foreground"
          >
            <FolderPlus className="size-4 shrink-0 text-muted-foreground" aria-hidden />
            <span className="flex-1">Open folder…</span>
          </button>
        </div>
      </PopoverContent>
    </Popover>
  );
}

/** The folder a project sits in, with the home directory as `~`. */
function shortParent(path: string): string {
  const parent = path.replace(/\/+$/, '').replace(/\/[^/]*$/, '') || '/';
  return parent.replace(/^\/(Users|home)\/[^/]+/, '~');
}
