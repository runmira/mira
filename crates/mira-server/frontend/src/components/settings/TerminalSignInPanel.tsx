import { lazy, Suspense, useEffect, useRef, useState } from 'react';
import { Check, Loader2, TerminalSquare } from 'lucide-react';
import { cancelTerminalAuth, getTerminalAuth, startTerminalAuth, type TerminalAuthView } from '../../api';
import { terminalSignInStatus } from '../../lib/agentAuth';

const TerminalView = lazy(() => import('../TerminalPanel').then((m) => ({ default: m.TerminalView })));

/** OpenCode's own sign-in, run with this instance's CLI in a terminal here.
 *  Credentials stay with OpenCode; Mira only refreshes readiness after it. */
export function TerminalSignInPanel({ instance, enabled, onRefresh }: { instance: string; enabled: boolean; onRefresh: () => void }) {
  const [view, setView] = useState<TerminalAuthView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const notify = useRef(onRefresh);
  notify.current = onRefresh;
  const active = view?.phase === 'running' || view?.phase === 'refreshing';

  useEffect(() => {
    let cancelled = false;
    void getTerminalAuth(instance).then((v) => !cancelled && setView(v)).catch(() => {});
    return () => { cancelled = true; };
  }, [instance]);

  // Poll only while something is happening; settle notifies the parent once.
  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    const timer = setInterval(() => {
      void getTerminalAuth(instance).then((next) => {
        if (cancelled) return;
        setView(next);
        if (next.phase === 'succeeded') notify.current();
      }).catch((e) => !cancelled && setError((e as Error).message));
    }, 1000);
    return () => { cancelled = true; clearInterval(timer); };
  }, [active, instance]);

  async function act(kind: 'start' | 'cancel') {
    setBusy(true);
    setError(null);
    try {
      setView(kind === 'start' ? await startTerminalAuth(instance) : await cancelTerminalAuth(instance));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const button = 'inline-flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1.5 text-xs hover:bg-muted disabled:opacity-50';
  const status = terminalSignInStatus(view, enabled);
  return (
    <div className="rounded-lg border border-border/60 p-3 text-xs" aria-label="OpenCode sign-in">
      <div className="flex items-center justify-between gap-2">
        <span className="font-medium">Sign-in</span>
        {view?.phase === 'running'
          ? <button type="button" className={button} disabled={busy} onClick={() => void act('cancel')}>Cancel</button>
          : <button type="button" className={button} disabled={busy || !enabled || view?.phase === 'refreshing'} onClick={() => void act('start')}>
              {busy ? <Loader2 className="size-3.5 animate-spin" /> : <TerminalSquare className="size-3.5" />}
              {view?.phase === 'succeeded' ? 'Sign in again' : 'Sign in in terminal'}
            </button>}
      </div>
      <div className="mt-2 inline-flex items-center gap-1.5" role="status">
        {active && <Loader2 className="size-3.5 animate-spin" />}
        {view?.phase === 'succeeded' && view.ready && !view.error && <Check className="size-3.5" />}
        {status}
      </div>
      {view?.phase === 'running' && view.terminal_id && (
        <div className="relative mt-2 h-64 overflow-hidden rounded-md border border-border bg-background">
          <Suspense fallback={null}>
            <TerminalView key={view.terminal_id} termId={view.terminal_id} visible onId={() => {}} attachOnly />
          </Suspense>
        </div>
      )}
      {view?.command && (view.phase === 'running' || view.phase === 'unsupported') && (
        <p className="mt-2 text-muted-foreground">Runs <code className="rounded bg-muted px-1 py-0.5">{view.command}</code></p>
      )}
      {(error || view?.error) && <p role="alert" className="mt-2 text-destructive">{error ?? view?.error}</p>}
      <p className="mt-2 text-muted-foreground">Uses this instance’s OpenCode CLI and home, so the login is the one its chats use.</p>
    </div>
  );
}
