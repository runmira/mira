import { useEffect, useState } from 'react';
import { Check, ChevronRight, CircleAlert, LoaderCircle, X } from 'lucide-react';
import { cn } from '../lib/utils';
export type WorkspaceSetup = { id: string; branch: string; phase: 'creating' | 'opening' | 'done' | 'failed' | 'cancelled'; startedAt: number; error?: string; failedStage?: 'creating' | 'opening'; retry?: () => void; cancel?: () => void; workLocally?: () => void };
export function reportWorkspaceSetup(setup: WorkspaceSetup) { window.dispatchEvent(new CustomEvent('mira:workspace-setup', { detail: setup })); }
/** Setup remains visible after the branch picker closes, in the shared timeline. */
export function WorkspaceSetupCard() {
  const [setup, setSetup] = useState<WorkspaceSetup | null>(null);
  const [open, setOpen] = useState(false);
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const receive = (event: Event) => { setSetup((event as CustomEvent<WorkspaceSetup>).detail); };
    window.addEventListener('mira:workspace-setup', receive);
    return () => window.removeEventListener('mira:workspace-setup', receive);
  }, []);
  const active = setup?.phase === 'creating' || setup?.phase === 'opening';
  useEffect(() => { if (!active) return; setNow(Date.now()); const timer = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(timer); }, [active]);
  if (!setup) return null;
  const label = setup.phase === 'creating' ? 'Creating worktree…' : setup.phase === 'opening' ? 'Opening workspace…' : setup.phase === 'failed' ? 'Workspace setup failed' : setup.phase === 'cancelled' ? 'Workspace opening cancelled' : 'Workspace ready';
  return <div className="py-2 text-[13px] text-muted-foreground">
    <div className="flex items-center gap-2"><button className="flex min-w-0 items-center gap-2" aria-expanded={open} onClick={() => setOpen(value => !value)}><ChevronRight className={cn('size-3.5', open && 'rotate-90')} />{active ? <LoaderCircle className="size-3.5 animate-spin" /> : setup.phase === 'failed' ? <CircleAlert className="size-3.5" /> : setup.phase === 'done' ? <Check className="size-3.5" /> : <X className="size-3.5" />}<span>{label}</span><span className="truncate opacity-70">{setup.branch}</span>{active && <span className="tabular-nums">{Math.max(0, Math.floor((now - setup.startedAt) / 1000))}s</span>}</button>
      {!active && <button className="ml-auto" aria-label="Dismiss workspace setup" onClick={() => setSetup(null)}><X className="size-3.5" /></button>}
    </div>
    {(open || setup.phase === 'failed') && <div className="ml-7 mt-2 space-y-1 text-xs"><p>{setup.phase === 'creating' ? 'Creating worktree' : setup.phase === 'cancelled' ? 'Opening cancelled. Creation may finish; any created worktree remains available in the picker.' : setup.phase === 'failed' && setup.failedStage !== 'opening' ? 'Could not create worktree' : 'Created worktree'}</p>{setup.phase !== 'creating' && setup.phase !== 'cancelled' && <p>{setup.phase === 'done' ? 'Opened workspace' : 'Open workspace'}</p>}{setup.error && <p role="alert">{setup.error}</p>}</div>}
    <div className="ml-7 mt-1 flex gap-3 text-xs">{setup.cancel && active && <button onClick={setup.cancel}>Cancel opening</button>}{setup.retry && setup.phase === 'failed' && <button onClick={setup.retry}>Retry</button>}{setup.workLocally && (setup.phase === 'creating' || setup.phase === 'failed') && <button onClick={setup.workLocally}>Work locally</button>}</div>
  </div>;
}
