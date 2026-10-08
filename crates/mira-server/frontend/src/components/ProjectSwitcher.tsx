import { useEffect, useMemo, useRef, useState } from 'react';
import { Check, Folder, Plus, Search } from 'lucide-react';
import { Dialog, DialogContent } from '@/components/ui/dialog';
import { listSessions } from '../api';
import { basename, cn, parentName } from '@/lib/utils';

/**
 * The project switcher: every folder Mira has a chat in, one keystroke
 * away, plus a way to add a new one.
 *
 * Where the *folder* picker answers "where is it on disk", this answers
 * "which one do I mean" — so the list is built from the chats that
 * already exist, most recent first, with the current project checked.
 * There is deliberately no "no project" row: this dialog is opened from
 * inside a project (the empty state names it), and leaving that context
 * is what the composer's own picker is for.
 *
 * Switching starts a **new** chat in that folder. The server
 * materialises a fresh slot per cwd, so this is the same swap the
 * project chip performs — just with the choice made from a list instead
 * of a file panel.
 */
export function ProjectSwitcher({
  open,
  onClose,
  cwd,
  onPicked,
  onNewProject,
}: {
  open: boolean;
  onClose: () => void;
  /** The chat's current folder; shown checked. */
  cwd: string;
  /** Fires with the chosen folder. The caller swaps the cwd, which the
   *  server turns into a new session. */
  onPicked: (path: string) => void;
  /** "New project" — hands off to the platform folder panel. */
  onNewProject: () => void;
}) {
  const [query, setQuery] = useState('');
  const [recent, setRecent] = useState<string[]>([]);
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (!open) return;
    setQuery('');
    // Focus after the dialog's enter animation, or the caret lands in a
    // field that is still scaling in and the first keystroke is lost.
    const t = setTimeout(() => inputRef.current?.focus(), 60);
    listSessions({ all: true })
      .then((all) => {
        // Most recently touched folder first; the list of chats is
        // already the honest answer to "what do I work in".
        const seen: string[] = [];
        for (const s of [...all].sort((a, b) => b.updated_at - a.updated_at)) {
          if (s.cwd && !seen.includes(s.cwd)) seen.push(s.cwd);
        }
        setRecent(seen);
      })
      .catch(() => setRecent([]));
    return () => clearTimeout(t);
  }, [open]);

  const projects = useMemo(() => {
    const all = [cwd, ...recent].filter(Boolean);
    const q = query.trim().toLowerCase();
    const matched = q
      ? all.filter((p) => p.toLowerCase().includes(q) || basename(p).toLowerCase().includes(q))
      : all;
    return matched.filter((p, i) => matched.indexOf(p) === i).slice(0, 40);
  }, [cwd, recent, query]);

  const pick = (path: string) => {
    onClose();
    onPicked(path);
  };

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent
        showClose={false}
        className="gap-0 overflow-hidden p-0 sm:max-w-[26rem]"
      >
        {/* Search */}
        <div className="flex items-center gap-2 px-4 py-3">
          <Search className="size-4 shrink-0 text-muted-foreground/60" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              // Enter opens the top match — the reason a search field in
              // a list dialog should never need the mouse.
              if (e.key === 'Enter' && projects.length > 0) pick(projects[0]);
              if (e.key === 'Escape') onClose();
            }}
            placeholder="Search projects"
            spellCheck={false}
            className="min-w-0 flex-1 bg-transparent text-[14px] text-foreground outline-none placeholder:text-muted-foreground/50"
          />
        </div>

        <div className="h-px bg-border/60" />

        {/* Projects */}
        <div className="max-h-[46vh] overflow-y-auto py-1">
          {projects.length === 0 && (
            <div className="px-4 py-3 text-[12.5px] text-muted-foreground">
              {query.trim() ? `No project matches “${query.trim()}”` : 'No projects yet'}
            </div>
          )}
          {projects.map((path) => {
            const active = path === cwd;
            return (
              <button
                key={path}
                type="button"
                onClick={() => pick(path)}
                title={path}
                className={cn(
                  'flex w-full items-center gap-3 px-4 py-2 text-left transition-colors',
                  active ? 'bg-accent/60' : 'hover:bg-fg/[0.05]',
                )}
              >
                <Folder className="size-4 shrink-0 text-muted-foreground/70" />
                <span
                  className={cn(
                    'min-w-0 flex-1 truncate text-[13.5px]',
                    active ? 'font-semibold text-foreground' : 'text-foreground/90',
                  )}
                >
                  {basename(path)}
                </span>
                {/* Parent folder, dimmed: two projects can share a name,
                    and the path is what tells them apart. */}
                <span className="max-w-[9rem] shrink-0 truncate text-[12px] text-muted-foreground/60">
                  {parentName(path)}
                </span>
                <span className="flex size-4 shrink-0 items-center justify-center">
                  {active && <Check className="size-4 text-mira-blue" />}
                </span>
              </button>
            );
          })}
        </div>

        <div className="h-px bg-border/60" />

        {/* New project */}
        <button
          type="button"
          onClick={() => {
            onClose();
            onNewProject();
          }}
          className="flex w-full items-center gap-3 px-4 py-2.5 text-left text-[13.5px] text-foreground/90 transition-colors hover:bg-fg/[0.05]"
        >
          <Plus className="size-4 shrink-0 text-muted-foreground/70" />
          <span className="flex-1">New project</span>
        </button>
      </DialogContent>
    </Dialog>
  );
}
