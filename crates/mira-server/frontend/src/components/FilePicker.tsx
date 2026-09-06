import { useEffect, useState } from 'react';
import { ArrowUp, File as FileIcon, Folder, HardDrive, House } from '@phosphor-icons/react';
import { type BrowseView } from '../api';
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { cn } from '@/lib/utils';

type Props = {
  open: boolean;
  startPath?: string;
  onClose: () => void;
  onPicked: (path: string) => void;
};

/** File-picker dialog. Sibling of FolderPicker but shows files too; a
 *  single click on a directory drills in, a single click on a file picks
 *  it and closes the dialog. Reuses /api/browse with include_files. */
export function FilePicker({ open, startPath, onClose, onPicked }: Props) {
  const [view, setView] = useState<BrowseView | null>(null);
  const [showHidden, setShowHidden] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [manual, setManual] = useState('');

  useEffect(() => {
    if (!open) return;
    setError(null);
    load(startPath);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  async function load(path: string | undefined) {
    try {
      // /api/browse currently only returns dirs by default. We pass a
      // parallel query for include_files via `showHidden`'s companion —
      // extend the call: build the URL manually.
      const params = new URLSearchParams();
      if (path) params.set('path', path);
      if (showHidden) params.set('show_hidden', 'true');
      params.set('include_files', 'true');
      const r = await fetch(`/api/browse?${params}`);
      if (!r.ok) throw new Error(`browse ${r.status}`);
      const v: BrowseView = await r.json();
      setView(v);
      setManual(v.path);
      setError(null);
    } catch (e) {
      setError(String((e as Error).message));
    }
  }

  useEffect(() => {
    if (!open || !view) return;
    load(view.path);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showHidden]);

  function pickFile(path: string) {
    onPicked(path);
    onClose();
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-[38rem] gap-3">
        <DialogHeader>
          <DialogTitle>Attach a file</DialogTitle>
        </DialogHeader>

        <div className="flex flex-wrap items-center gap-1.5">
          {view?.home && (
            <Button variant="outline" size="sm" onClick={() => load(view.home!)}>
              <House className="size-3.5" /> Home
            </Button>
          )}
          <Button variant="outline" size="sm" onClick={() => load('/')}>
            <HardDrive className="size-3.5" /> /
          </Button>
          {view?.parent && (
            <Button variant="outline" size="sm" onClick={() => load(view.parent!)}>
              <ArrowUp className="size-3.5" /> up
            </Button>
          )}
          <label className="ml-auto inline-flex items-center gap-1.5 text-xs text-muted-foreground">
            <input
              type="checkbox"
              checked={showHidden}
              onChange={(e) => setShowHidden(e.target.checked)}
              className="accent-mira-blue"
            />
            show hidden
          </label>
        </div>

        <Input
          value={manual}
          onChange={(e) => setManual(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); load(manual.trim()); } }}
          spellCheck={false}
          placeholder="/absolute/path or ~/relative"
          className="font-mono text-xs"
        />

        {error && <div className="text-xs text-destructive font-mono">{error}</div>}

        <div className="max-h-[42vh] overflow-y-auto rounded-md border border-border/60 bg-background p-1">
          {view?.entries.length === 0 && !error && (
            <div className="px-2 py-2 text-xs text-muted-foreground">Empty folder</div>
          )}
          {view?.entries.map((e) => (
            <button
              key={e.path}
              className={cn(
                'flex w-full items-center gap-2 rounded px-2.5 py-1.5 text-left text-sm transition-colors cursor-pointer',
                'text-foreground hover:bg-accent/10',
              )}
              onClick={() => (e.is_dir ? load(e.path) : pickFile(e.path))}
              title={e.path}
            >
              {e.is_dir ? (
                <Folder className="size-3.5 shrink-0 text-mira-blue" />
              ) : (
                <FileIcon className="size-3.5 shrink-0 text-muted-foreground/70" />
              )}
              <span className="truncate">{e.name}</span>
            </button>
          ))}
          {view?.truncated && (
            <div className="px-2 py-1 text-xs text-muted-foreground">… list truncated at 500 entries</div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose}>Cancel</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
