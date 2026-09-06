import { useEffect, useState } from 'react';
import { ArrowUp, Folder, House, HardDrive } from '@phosphor-icons/react';
import { browse, putCwd, type BrowseView } from '../api';
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { cn } from '@/lib/utils';

type Props = {
  open: boolean;
  onClose: () => void;
  onPicked: (path: string) => void;
};

export function FolderPicker({ open, onClose, onPicked }: Props) {
  const [view, setView] = useState<BrowseView | null>(null);
  const [showHidden, setShowHidden] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [manual, setManual] = useState('');

  useEffect(() => {
    if (!open) return;
    setError(null);
    load(undefined);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  async function load(path: string | undefined) {
    try {
      const v = await browse(path, showHidden);
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

  async function pick(path: string) {
    setBusy(true);
    try {
      await putCwd(path);
      onPicked(path);
      onClose();
    } catch (e) {
      setError(String((e as Error).message));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-[38rem] gap-3">
        <DialogHeader>
          <DialogTitle>Choose project folder</DialogTitle>
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

        <div className="max-h-[40vh] overflow-y-auto rounded-md border border-border/60 bg-background p-1">
          {view?.entries.length === 0 && !error && (
            <div className="px-2 py-2 text-xs text-muted-foreground">Empty folder</div>
          )}
          {view?.entries.map((e) => (
            <button
              key={e.path}
              className={cn(
                'flex w-full items-center gap-2 rounded px-2.5 py-1.5 text-left text-sm transition-colors',
                e.is_dir
                  ? 'text-foreground hover:bg-accent/10 cursor-pointer'
                  : 'text-muted-foreground/60 cursor-default',
              )}
              onClick={() => e.is_dir && load(e.path)}
              onDoubleClick={() => e.is_dir && load(e.path)}
              disabled={!e.is_dir}
              title={e.path}
            >
              <Folder className={cn('size-3.5 shrink-0', e.is_dir ? 'text-mira-blue' : 'text-muted-foreground/40')} />
              <span className="truncate">{e.name}</span>
            </button>
          ))}
          {view?.truncated && (
            <div className="px-2 py-1 text-xs text-muted-foreground">… list truncated at 500 entries</div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={busy}>Cancel</Button>
          <Button onClick={() => view && pick(view.path)} disabled={busy || !view} title={view?.path}>
            {busy ? 'Setting…' : 'Select this folder'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
