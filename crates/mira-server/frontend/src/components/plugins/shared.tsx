import { useState, type ReactNode } from 'react';
import { AlertCircle, Ellipsis } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent } from '@/components/ui/dialog';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';

/** Stable hue from a name, for letter avatars. */
function hue(name: string): number {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) % 360;
  return h;
}

/** Plugin / server icon: shows an image when `src` is provided, falls back to a letter tile. */
export function Avatar({ name, size = 'md', src }: { name: string; size?: 'sm' | 'md' | 'lg'; src?: string | null }) {
  const [imgFailed, setImgFailed] = useState(false);
  const h = hue(name);
  const letter = (name.replace(/^plugin:/, '').match(/[a-z0-9]/i)?.[0] ?? '?').toUpperCase();
  const sizeClass = cn(
    'shrink-0 rounded-xl',
    size === 'sm' && 'size-7',
    size === 'md' && 'size-10',
    size === 'lg' && 'size-14',
  );
  if (src && !imgFailed) {
    return (
      <img
        aria-hidden
        src={src}
        alt=""
        className={cn(sizeClass, 'object-cover')}
        onError={() => setImgFailed(true)}
      />
    );
  }
  return (
    <div
      aria-hidden
      className={cn(
        sizeClass,
        'flex items-center justify-center font-semibold',
        size === 'sm' && 'text-[11px]',
        size === 'md' && 'text-[15px]',
        size === 'lg' && 'text-[20px]',
      )}
      style={{
        background: `hsl(${h} 40% 18%)`,
        color: `hsl(${h} 75% 72%)`,
        boxShadow: `inset 0 0 0 1px hsl(${h} 50% 32% / 0.6)`,
      }}
    >
      {letter}
    </div>
  );
}

export function Switch({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={(e) => {
        e.stopPropagation();
        onChange(!checked);
      }}
      className={cn(
        'relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors',
        'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50',
        checked ? 'bg-mira-purple' : 'bg-white/15',
      )}
    >
      <span
        className={cn(
          'inline-block size-4 rounded-full bg-white shadow transition-transform',
          checked ? 'translate-x-[18px]' : 'translate-x-0.5',
        )}
      />
    </button>
  );
}

export function Pill({
  children,
  tone = 'neutral',
  title,
  className,
}: {
  children: ReactNode;
  tone?: 'neutral' | 'green' | 'amber' | 'red' | 'purple' | 'blue';
  title?: string;
  className?: string;
}) {
  const tones = {
    neutral: 'border-border/60 bg-white/5 text-muted-foreground',
    green:   'border-emerald-500/25 bg-emerald-500/[0.08] text-emerald-300',
    amber:   'border-amber-500/25 bg-amber-500/[0.08] text-amber-300',
    red:     'border-destructive/40 bg-destructive/10 text-destructive',
    purple:  'border-mira-purple/25 bg-mira-purple/[0.08] text-mira-purple',
    blue:    'border-mira-blue/25 bg-mira-blue/[0.08] text-mira-blue',
  } as const;
  return (
    <span
      title={title}
      className={cn(
        'inline-flex items-center gap-1 whitespace-nowrap rounded-full border px-2 py-0.5 text-[11px] leading-4',
        tones[tone],
        className,
      )}
    >
      {children}
    </span>
  );
}

export type MenuItem = {
  label: string;
  icon?: ReactNode;
  onSelect: () => void;
  danger?: boolean;
  disabled?: boolean;
};

export function RowMenu({ items, label = 'More actions' }: { items: MenuItem[]; label?: string }) {
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={label}
          onClick={(e) => e.stopPropagation()}
          className={cn(
            'shrink-0 rounded-lg p-1.5 text-muted-foreground/60 transition-colors hover:bg-white/8 hover:text-foreground',
            open && 'bg-white/8 text-foreground',
          )}
        >
          <Ellipsis className="size-4" />
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-52 p-1" align="end" onClick={(e) => e.stopPropagation()}>
        <div className="flex flex-col">
          {items.map((it) => (
            <button
              key={it.label}
              type="button"
              disabled={it.disabled}
              onClick={() => {
                setOpen(false);
                it.onSelect();
              }}
              className={cn(
                'flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] transition-colors',
                it.disabled
                  ? 'cursor-not-allowed text-muted-foreground/40'
                  : it.danger
                    ? 'text-destructive hover:bg-destructive/10'
                    : 'text-foreground hover:bg-accent/60',
              )}
            >
              {it.icon && <span className="shrink-0 text-muted-foreground [&_svg]:size-3.5">{it.icon}</span>}
              {it.label}
            </button>
          ))}
        </div>
      </PopoverContent>
    </Popover>
  );
}

export function ConfirmDialog({
  title,
  body,
  confirmLabel,
  busy,
  onCancel,
  onConfirm,
}: {
  title: string;
  body: ReactNode;
  confirmLabel: string;
  busy?: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <Dialog open onOpenChange={(o) => !o && onCancel()}>
      <DialogContent className="max-w-md">
        <div className="text-[15px] font-semibold">{title}</div>
        <div className="text-[13px] text-muted-foreground">{body}</div>
        <div className="flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={onCancel}>
            Cancel
          </Button>
          <Button variant="destructive" size="sm" disabled={busy} onClick={onConfirm}>
            {busy ? 'Working…' : confirmLabel}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

export function ErrorBanner({ message, onDismiss }: { message: string; onDismiss?: () => void }) {
  return (
    <div className="flex items-start gap-2 rounded-xl border border-destructive/30 bg-destructive/8 px-3 py-2.5 text-[13px] text-destructive">
      <AlertCircle className="mt-0.5 size-4 shrink-0" />
      <span className="min-w-0 flex-1 break-words">{message}</span>
      {onDismiss && (
        <button type="button" onClick={onDismiss} className="shrink-0 text-destructive/70 hover:text-destructive">
          Dismiss
        </button>
      )}
    </div>
  );
}

export function EmptyState({
  icon,
  title,
  body,
  action,
}: {
  icon: ReactNode;
  title: string;
  body: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center justify-center gap-3 rounded-2xl border border-dashed border-border/60 bg-white/[0.02] px-6 py-16 text-center">
      <div className="rounded-2xl border border-border/60 bg-white/5 p-3.5 text-muted-foreground [&_svg]:size-5">
        {icon}
      </div>
      <div className="text-[14.5px] font-semibold">{title}</div>
      <div className="max-w-md text-[13px] leading-relaxed text-muted-foreground">{body}</div>
      {action}
    </div>
  );
}

/** Tracks which action is running (by key) and the last error. */
export function useActions() {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  async function run<T>(key: string, fn: () => Promise<T>): Promise<T | undefined> {
    setBusy(key);
    setError(null);
    try {
      return await fn();
    } catch (e) {
      setError((e as Error).message);
      return undefined;
    } finally {
      setBusy(null);
    }
  }
  return { busy, error, setError, run };
}

export function relativeTime(secs: number): string {
  if (!secs) return '';
  const d = Date.now() / 1000 - secs;
  if (d < 60) return 'just now';
  if (d < 3600) return `${Math.floor(d / 60)}m ago`;
  if (d < 86400) return `${Math.floor(d / 3600)}h ago`;
  return `${Math.floor(d / 86400)}d ago`;
}

export function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? '' : 's'}`;
}
