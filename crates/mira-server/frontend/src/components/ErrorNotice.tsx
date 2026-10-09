import { AlertCircle, RotateCcw } from 'lucide-react';

export function ErrorNotice({ title, description, details, onRetry, retryLabel = 'Retry', pending = false, disabled = false, secondary }: {
  title: string; description: string; details: string; onRetry?: () => void;
  retryLabel?: string; pending?: boolean; disabled?: boolean;
  secondary?: { label: string; onClick: () => void };
}) {
  return <section role="alert" className="my-2 rounded-xl border border-border bg-background px-3 py-3 text-foreground">
    <div className="flex items-start gap-2">
      <AlertCircle aria-hidden className="mt-0.5 size-4 shrink-0 text-amber-700 dark:text-amber-400" />
      <div className="min-w-0 flex-1">
        <p className="text-[12.5px] font-semibold leading-5">{title}</p>
        <p className="mt-0.5 text-[12px] leading-5 text-muted-foreground">{description}</p>
        {(onRetry || secondary) && <div className="mt-2 flex flex-wrap items-center gap-2">
          {onRetry && <button type="button" disabled={pending || disabled} onClick={onRetry} className="inline-flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1.5 text-[12px] font-medium hover:bg-secondary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-mira-blue disabled:opacity-50"><RotateCcw aria-hidden className={pending ? 'size-3 animate-spin' : 'size-3'} />{pending ? 'Retrying…' : retryLabel}</button>}
          {secondary && <button type="button" onClick={secondary.onClick} className="rounded-lg px-2 py-1.5 text-[12px] text-muted-foreground hover:bg-secondary hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-mira-blue">{secondary.label}</button>}
        </div>}
        <details className="mt-2 text-[11px] text-muted-foreground"><summary className="w-fit cursor-pointer rounded focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-mira-blue">Technical details</summary><pre className="mt-2 max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-secondary/50 p-2 font-mono text-[11px]">{details}</pre></details>
      </div>
    </div>
  </section>;
}
