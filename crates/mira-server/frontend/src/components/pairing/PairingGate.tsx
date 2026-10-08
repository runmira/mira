import { useEffect, useState } from 'react';
import { getPairingStatus } from '../../lib/pairing';
import { lazyNamed } from '../../lib/lazy';
import { LazyBoundary } from '../LazyBoundary';

// Only unpaired devices need the pair screen; keep it out of the first load.
const PairScreen = lazyNamed(() => import('./PairScreen'), 'PairScreen');

/**
 * Sits in front of everything. This computer and paired devices go straight
 * through; any other device gets the pair screen. Scanning the QR code in
 * Settings → Devices opens `/?pair=CODE`, which pairs without typing.
 */
export function PairingGate({ children }: { children: React.ReactNode }) {
  const [state, setState] = useState<'checking' | 'ok' | 'pair'>('checking');

  useEffect(() => {
    let live = true;
    getPairingStatus()
      .then((s) => live && setState(s.local || s.paired ? 'ok' : 'pair'))
      // Server unreachable: let the app show its own connection state.
      .catch(() => live && setState('ok'));
    return () => {
      live = false;
    };
  }, []);

  if (state === 'checking') return null;
  if (state === 'pair') {
    return (
      <LazyBoundary>
        <PairScreen />
      </LazyBoundary>
    );
  }
  return <>{children}</>;
}

