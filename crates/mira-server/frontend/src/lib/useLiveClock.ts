import { useEffect, useState } from 'react';
/** Only the component displaying elapsed time wakes once per visible second. */
export function useLiveClock(enabled: boolean) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!enabled) return;
    let timer: ReturnType<typeof setInterval> | undefined;
    const visibility = () => {
      if (timer) clearInterval(timer);
      timer = undefined;
      if (!document.hidden) { setNow(Date.now()); timer = setInterval(() => setNow(Date.now()), 1000); }
    };
    visibility(); document.addEventListener('visibilitychange', visibility);
    return () => { if (timer) clearInterval(timer); document.removeEventListener('visibilitychange', visibility); };
  }, [enabled]);
  return now;
}
