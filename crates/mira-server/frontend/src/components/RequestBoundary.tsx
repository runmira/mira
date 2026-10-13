import type { ReactNode } from 'react';
import { useSelector } from '@tanstack/react-store';
import { useAppRuntime } from '../state/RuntimeProvider';
import { selectActiveSession } from '../state/runtimeStore';
export function RequestBoundary({ id, children }: { id: string; children: ReactNode }) {
  const runtime = useAppRuntime();
  const delivery = useSelector(runtime.sessionState, state => selectActiveSession(state).requestDelivery[id]);
  return (
    <fieldset disabled={delivery?.status === 'submitting'} className="m-0 min-w-0 border-0 p-0">
      {children}
      {delivery?.status === 'submitting' && <p role="status" className="px-2 py-1 text-xs text-muted-foreground">Waiting for server confirmation…</p>}
      {delivery?.error && <p role="alert" className="px-2 py-1 text-xs text-destructive">{delivery.error}</p>}
    </fieldset>
  );
}
