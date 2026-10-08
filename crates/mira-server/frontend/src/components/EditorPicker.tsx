/**
 * "Open with" editor picker + shared editor icons.
 *
 * Trigger shows the preferred editor's real app icon; the menu lists
 * locally-detected editors with icon tiles, kind subtext and a check on
 * the preferred one. Picking an editor makes it preferred and opens the
 * session folder in it. Icons come from the server (real .icns art via
 * `/api/editors/icon/:id`) with site favicons as fallback.
 */
import { useEffect, useState } from 'react';
import { Check, ChevronDown, FolderOpen, LoaderCircle, Settings2 } from 'lucide-react';

import { cn } from '@/lib/utils';
import { PREF_KEYS, useStringPref } from '@/lib/prefs';
import {
  editorIconUrl,
  listEditors,
  openInEditor,
  type EditorEntry,
} from '@/lib/editors';
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover';

export function EditorIcon({
  entry,
  className,
  tile = false,
}: {
  entry: Pick<EditorEntry, 'id' | 'has_icon'>;
  className?: string;
  /** Wrap the mark in a subtle tile (menu rows). */
  tile?: boolean;
}) {
  const [failed, setFailed] = useState(false);
  const url = failed ? null : editorIconUrl(entry);
  const mark = url ? (
    <img
      src={url}
      alt=""
      aria-hidden="true"
      draggable={false}
      onError={() => setFailed(true)}
      className="size-full object-contain"
    />
  ) : null;
  if (!tile) {
    return (
      <span className={cn('inline-flex size-5 shrink-0 items-center justify-center', className)}>
        {mark}
      </span>
    );
  }
  return (
    <span
      className={cn(
        'inline-flex size-8 shrink-0 items-center justify-center overflow-hidden rounded-lg',
        'bg-fg/[0.04] ring-1 ring-fg/10',
        className,
      )}
    >
      <span className="inline-flex size-[22px] items-center justify-center">{mark}</span>
    </span>
  );
}

function kindLabel(entry: EditorEntry): string {
  if (entry.kind === 'system') return entry.id === 'terminal' ? 'Terminal' : 'File manager';
  if (entry.kind === 'app') return 'Application';
  return 'Command line';
}

export function EditorPicker({ cwd, onOpenSettings }: { cwd: string; onOpenSettings?: () => void }) {
  const [preferred, setPreferred] = useStringPref(PREF_KEYS.preferredEditor, '');
  const [entries, setEntries] = useState<EditorEntry[] | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    listEditors()
      .then((v) => {
        if (!cancelled) setEntries(v.editors);
      })
      .catch(() => {
        if (!cancelled) setEntries([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const current = entries?.find((e) => e.id === preferred) ?? null;

  async function pick(entry: EditorEntry) {
    setPreferred(entry.id);
    setBusyId(entry.id);
    setError(null);
    try {
      await openInEditor(cwd, entry.id);
      setOpen(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          title={current ? `Open in ${current.name}` : 'Open in an editor'}
          aria-label="Open session folder in an editor"
          className="group flex h-8 shrink-0 items-center gap-1 rounded-lg border border-border/60 elev-card dark:bg-secondary/40 py-1 pl-1 pr-1.5 transition-colors hover:border-border dark:hover:bg-secondary"
        >
          <span className="inline-flex size-6 items-center justify-center overflow-hidden rounded-md bg-fg/[0.04] ring-1 ring-fg/10">
            {current ? (
              <EditorIcon entry={current} className="size-[18px]" />
            ) : (
              <FolderOpen className="size-3.5 text-muted-foreground" strokeWidth={1.75} />
            )}
          </span>
          <ChevronDown className="size-3 text-muted-foreground transition-transform group-data-[state=open]:rotate-180" />
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-72 p-1.5">
        <div className="px-2 pb-1.5 pt-1">
          <div className="text-[12.5px] font-semibold text-foreground">Open with</div>
          <div className="truncate font-mono text-[10.5px] text-muted-foreground/70">
            {cwd}
          </div>
        </div>
        {entries === null ? (
          <div className="flex flex-col gap-1 px-1 py-1">
            {[0, 1, 2].map((i) => (
              <div key={i} className="flex items-center gap-2.5 rounded-lg px-2 py-1.5">
                <div className="size-8 shrink-0 animate-pulse rounded-lg bg-secondary" />
                <div className="flex min-w-0 flex-1 flex-col gap-1">
                  <div className="h-3 w-2/3 animate-pulse rounded bg-secondary" />
                  <div className="h-2 w-1/3 animate-pulse rounded bg-secondary/60" />
                </div>
              </div>
            ))}
          </div>
        ) : entries.length === 0 ? (
          <div className="px-3 py-3 text-[12.5px] text-muted-foreground">
            No editors detected on this machine.
          </div>
        ) : (
          entries.map((entry) => {
            const selected = entry.id === preferred;
            const busy = busyId === entry.id;
            return (
              <button
                key={entry.id}
                type="button"
                disabled={busy}
                onClick={() => void pick(entry)}
                className={cn(
                  'flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left transition-colors disabled:opacity-60',
                  selected ? 'bg-accent/70' : 'hover:bg-accent/60',
                )}
              >
                {busy ? (
                  <span className="inline-flex size-8 shrink-0 items-center justify-center rounded-lg bg-fg/[0.04] ring-1 ring-fg/10">
                    <LoaderCircle className="size-4 animate-spin text-muted-foreground" />
                  </span>
                ) : (
                  <EditorIcon entry={entry} tile />
                )}
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[13px] font-medium text-foreground">
                    {entry.name}
                  </span>
                  <span className="block text-[11px] text-muted-foreground/75">
                    {kindLabel(entry)}
                  </span>
                </span>
                {selected && <Check className="size-3.5 shrink-0 text-mira-blue" strokeWidth={2.5} />}
              </button>
            );
          })
        )}
        {error && (
          <div className="mx-1 mb-1 mt-1 rounded-lg bg-destructive/10 px-2.5 py-1.5 text-[11.5px] text-destructive">
            {error}
          </div>
        )}
        {onOpenSettings && (
          <>
            <div className="mx-1 my-1 h-px bg-border/50" />
            <button
              type="button"
              onClick={() => {
                setOpen(false);
                onOpenSettings();
              }}
              className="flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left text-[12.5px] text-muted-foreground transition-colors hover:bg-accent/60 hover:text-foreground"
            >
              <Settings2 className="size-4 shrink-0" />
              Preferred editor settings
            </button>
          </>
        )}
      </PopoverContent>
    </Popover>
  );
}
