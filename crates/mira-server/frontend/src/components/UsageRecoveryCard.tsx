import { useEffect, useState } from 'react';
import { AlarmClock, ArrowRight, BellOff, Check, Gauge, LoaderCircle, X } from 'lucide-react';
import { motion, useReducedMotion } from 'framer-motion';
import type { QueuedInput } from '../types';

export type RecoveryAction = 'schedule' | 'retry' | 'cancel' | 'snooze' | 'show' | 'dismiss';
export function UsageRecoveryCard({ item, disabled, onAction }: { item: QueuedInput; disabled: boolean; onAction: (id: string, action: RecoveryAction) => Promise<void> }) {
  const [now, setNow] = useState(Date.now());
  const [pending, setPending] = useState<RecoveryAction | null>(null);
  const [error, setError] = useState<string | null>(null);
  const reduced = useReducedMotion();
  const recovery = item.recovery!;
  const scheduled = recovery.scheduled_at != null;
  const future = recovery.reset_at != null && recovery.reset_at > now;
  useEffect(() => {
    if (!recovery.reset_at || recovery.reset_at <= Date.now()) return;
    const timer = setTimeout(() => setNow(Date.now()), Math.min(60_000, recovery.reset_at - Date.now()));
    return () => clearTimeout(timer);
  }, [now, recovery.reset_at]);
  async function act(action: RecoveryAction) {
    setPending(action); setError(null);
    try { await onAction(item.id, action); } catch (e) { setError((e as Error).message); }
    finally { setPending(null); }
  }
  const muted = recovery.snoozed && future;
  return <motion.section aria-label="Usage limit recovery" initial={reduced ? false : { opacity: 0, y: 5 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: .18 }} className="mt-2 overflow-hidden rounded-2xl border border-border bg-background text-foreground">
    <div className="flex items-start gap-3 p-4">
      <span className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-amber-500/10 text-amber-700 dark:text-amber-400"><Gauge className="size-[18px]" /></span>
      <div className="min-w-0 flex-1">
        <h3 className="text-[13px] font-semibold leading-5">{item.dispatching ? 'Resuming your task' : scheduled ? 'Your task will resume automatically' : muted ? 'Usage limit notice snoozed' : 'Usage limit reached'}</h3>
        <p className="mt-1 text-[12px] leading-5 text-muted-foreground">{item.error ?? (item.dispatching ? 'Checking existing work before continuing.' : recovery.reset_at ? `${future ? 'Resets' : 'Reset was reported for'} ${new Date(recovery.reset_at).toLocaleString([], { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })}.` : 'This provider did not report a reset time. Retry when your allowance is available.')}</p>
        {scheduled && !item.dispatching && <p className="mt-1 flex items-center gap-1.5 text-[11px] text-muted-foreground"><Check className="size-3" /> Saved for this chat · continues in the background while Mira is running</p>}
      </div>
      {scheduled && !item.dispatching && <AlarmClock className="mt-1 size-4 shrink-0 text-amber-700 dark:text-amber-400" />}
    </div>
    {!item.dispatching && !item.error && <div className="flex flex-wrap items-center gap-2 border-t border-border px-4 py-3">
      {scheduled ? <button disabled={disabled || !!pending} onClick={() => void act('cancel')} className="inline-flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-[12px] font-medium hover:bg-secondary disabled:opacity-50"><X className="size-3.5" />Cancel auto-resume</button> : muted ? <button disabled={disabled || !!pending} onClick={() => void act('show')} className="text-[12px] font-medium hover:underline">Show recovery options</button> : <>
        {future && <button disabled={disabled || !!pending} onClick={() => void act('schedule')} className="inline-flex items-center gap-1.5 rounded-lg bg-foreground px-3 py-1.5 text-[12px] font-medium text-background disabled:opacity-50"><AlarmClock className="size-3.5" />Resume at reset</button>}
        <button disabled={disabled || !!pending} onClick={() => void act('retry')} className="inline-flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-[12px] font-medium hover:bg-secondary disabled:opacity-50">Retry now<ArrowRight className="size-3.5" /></button>
        {future && <button disabled={disabled || !!pending} onClick={() => void act('snooze')} className="ml-auto inline-flex items-center gap-1.5 text-[12px] text-muted-foreground hover:text-foreground disabled:opacity-50"><BellOff className="size-3.5" />Snooze until reset</button>}
      </>}
      {pending && <LoaderCircle aria-label="Saving recovery" className="size-3.5 animate-spin" />}
    </div>}
    {item.error && <div className="border-t border-border px-4 py-3"><p className="mb-2 text-[12px] text-muted-foreground">Check the transcript before sending a new message. Automatic delivery is paused.</p><button disabled={disabled || !!pending} onClick={() => void act('dismiss')} className="text-[12px] font-medium hover:underline">Dismiss recovery</button></div>}
    {(error || disabled) && <p role={error ? 'alert' : undefined} className="px-4 pb-3 text-[12px] text-muted-foreground">{error ?? 'Reconnect or wait for the active turn to finish to change recovery.'}</p>}
  </motion.section>;
}
