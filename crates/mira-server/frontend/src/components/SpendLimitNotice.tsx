import { useEffect, useState } from 'react';
import type { SpendStatus } from '../types';

export function SpendLimitNotice({ sessionId, busy }: { sessionId?: string | null; busy?: boolean }) {
  const [status, setStatus] = useState<SpendStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [raising, setRaising] = useState(false);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    setStatus(null); setError(null);
    if (!sessionId) return;
    const controller = new AbortController();
    const url = `/api/sessions/${encodeURIComponent(sessionId)}/spend`;
    async function refresh() {
      try {
        const response = await fetch(url, { signal: controller.signal, cache: 'no-store' });
        if (!response.ok) throw new Error('Could not check spend limits.');
        const next: SpendStatus = await response.json();
        if (!controller.signal.aborted) { setStatus(next); setError(null); }
      } catch (e) { if (!controller.signal.aborted) setError(e instanceof Error ? e.message : String(e)); }
    }
    void refresh();
    const timer = setInterval(() => void refresh(), busy ? 2000 : 10000);
    return () => { controller.abort(); clearInterval(timer); };
  }, [sessionId, busy, revision]);
  const reached = status?.limits.filter(v => v.warning) ?? [];
  const sessionCaps = status?.limits.filter(v => v.scope.endsWith('session')) ?? [];
  const base = Math.max(0, ...sessionCaps.map(v => Math.max(v.spent, v.limit)));
  const next = Math.ceil(Math.max(base * 1.25, base + 1) * 100) / 100;
  const canRaise = reached.some(v => v.scope.endsWith('session'));
  async function raise() {
    if (!sessionId || raising) return;
    setRaising(true); setError(null);
    try {
      const response = await fetch(`/api/sessions/${encodeURIComponent(sessionId)}/spend/raise`, { method: 'POST' });
      if (!response.ok) { const body = await response.json(); throw new Error(body.error ?? 'Could not raise the limit.'); }
      setRevision(v => v + 1);
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setRaising(false); }
  }
  if (!reached.length && !error && !status?.message) return null;
  return <div role="status" className="mx-3 mb-2 rounded-md border border-border bg-muted/40 px-3 py-2 text-xs">
    {status?.message && <p>{status.message}</p>}
    {reached.map(v => <p key={v.scope}>{v.blocked ? 'Spend limit reached' : '80% spend warning'}: {v.scope}, ${v.spent.toFixed(2)} of ${v.limit.toFixed(2)}.</p>)}
    {status?.blocked && <p className="mt-1 text-muted-foreground">Work is saved. Raise the session limit to continue, or change daily and monthly caps in Settings.</p>}
    {canRaise && <button type="button" disabled={raising} onClick={() => void raise()} className="mt-2 rounded border px-2 py-1 disabled:opacity-50">{raising ? 'Raising…' : `Raise the limit for this session to $${next.toFixed(2)}`}</button>}
    {error && <p role="alert" className="text-destructive">{error}</p>}
  </div>;
}
