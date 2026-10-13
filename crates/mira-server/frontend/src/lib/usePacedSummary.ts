import { useLayoutEffect, useRef, useState } from 'react';
import { useReducedMotion } from 'framer-motion';
import { PREF_KEYS, useBoolPref } from './prefs';
import { createSummaryPlayback, type SummarySnapshot } from './summaryPlayback';

export function usePacedSummary<T>(snapshot: SummarySnapshot<T>, active: boolean) {
  const reduced = useReducedMotion();
  const [preferReduced] = useBoolPref(PREF_KEYS.reduceMotion, false);
  const enabled = active && !reduced && !preferReduced;
  const latest = useRef(snapshot); latest.current = snapshot;
  const [displayed, setDisplayed] = useState(snapshot);
  const service = useRef<ReturnType<typeof createSummaryPlayback<T>> | null>(null);
  useLayoutEffect(() => {
    const controller = createSummaryPlayback(latest.current, setDisplayed);
    service.current = controller;
    return () => { controller.dispose(); service.current = null; };
  }, []);
  useLayoutEffect(() => service.current?.update(snapshot, enabled), [snapshot.key, snapshot.version, enabled]);
  // Completion and reduced motion converge immediately, even before effects run.
  return enabled ? displayed : snapshot;
}
