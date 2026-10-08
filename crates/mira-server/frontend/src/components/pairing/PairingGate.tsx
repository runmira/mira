import { useEffect, useRef, useState } from 'react';
import { Check, Loader2, MonitorSmartphone } from 'lucide-react';
import { cn } from '@/lib/utils';
import miraLogo from '../../assets/mira-logo.png';
import { getPairingStatus, guessDeviceName, pairThisDevice } from '../../lib/pairing';

const CODE_LEN = 8;
/** Matches the server's alphabet: no 0/O or 1/I/L. */
const ALLOWED = /[2-9A-HJ-NP-Z]/;

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
  if (state === 'pair') return <PairScreen />;
  return <>{children}</>;
}

function codeFromUrl(): string {
  const raw = new URLSearchParams(window.location.search).get('pair') ?? '';
  return [...raw.toUpperCase()].filter((c) => ALLOWED.test(c)).join('').slice(0, CODE_LEN);
}

function PairScreen() {
  const [chars, setChars] = useState<string[]>(() => {
    const c = codeFromUrl();
    return Array.from({ length: CODE_LEN }, (_, i) => c[i] ?? '');
  });
  const [name, setName] = useState(() => guessDeviceName());
  const [status, setStatus] = useState<'idle' | 'pairing' | 'paired' | 'error'>('idle');
  const [error, setError] = useState<string | null>(null);
  const inputs = useRef<(HTMLInputElement | null)[]>([]);
  const code = chars.join('');

  // The code shouldn't linger in history once it's been read.
  useEffect(() => {
    const url = new URL(window.location.href);
    if (url.searchParams.has('pair')) {
      url.searchParams.delete('pair');
      window.history.replaceState(null, '', url.pathname + url.search + url.hash);
    }
  }, []);

  useEffect(() => {
    if (code.length === CODE_LEN && status === 'idle') void submit(code);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [code]);

  useEffect(() => {
    const first = chars.findIndex((c) => !c);
    inputs.current[first === -1 ? CODE_LEN - 1 : first]?.focus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function submit(full: string) {
    setStatus('pairing');
    setError(null);
    try {
      await pairThisDevice(full, name);
      setStatus('paired');
      window.setTimeout(() => window.location.reload(), 900);
    } catch (e) {
      setStatus('error');
      setError((e as Error).message);
      setChars(Array(CODE_LEN).fill(''));
      window.setTimeout(() => inputs.current[0]?.focus(), 0);
    }
  }

  function fill(from: number, text: string) {
    const clean = [...text.toUpperCase()].filter((c) => ALLOWED.test(c)).slice(0, CODE_LEN - from);
    if (!clean.length) return;
    setStatus((s) => (s === 'error' ? 'idle' : s));
    setChars((prev) => {
      const next = [...prev];
      clean.forEach((c, k) => {
        if (from + k < CODE_LEN) next[from + k] = c;
      });
      return next;
    });
    inputs.current[Math.min(from + clean.length, CODE_LEN - 1)]?.focus();
  }

  function onKey(i: number, e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === 'Backspace') {
      e.preventDefault();
      setChars((prev) => {
        const next = [...prev];
        const at = prev[i] ? i : Math.max(0, i - 1);
        next[at] = '';
        inputs.current[at]?.focus();
        return next;
      });
    } else if (e.key === 'ArrowLeft') {
      inputs.current[Math.max(0, i - 1)]?.focus();
    } else if (e.key === 'ArrowRight') {
      inputs.current[Math.min(CODE_LEN - 1, i + 1)]?.focus();
    }
  }

  const busy = status === 'pairing' || status === 'paired';

  return (
    <div className="relative flex min-h-screen items-center justify-center overflow-hidden bg-background px-4">
      {/* Soft glow behind the card. */}
      <div
        aria-hidden
        className="pointer-events-none absolute left-1/2 top-1/2 size-[560px] -translate-x-1/2 -translate-y-1/2 rounded-full bg-mira-blue/10 blur-3xl"
      />
      <div className="relative w-full max-w-[420px] rounded-3xl border border-border/60 bg-card/70 p-7 shadow-[0_30px_80px_-30px_rgba(0,0,0,0.6)] backdrop-blur-xl sm:p-9">
        <div className="mb-7 flex items-center gap-3">
          <img src={miraLogo} alt="" className="size-10 rounded-xl" />
          <div className="flex items-center gap-1.5 text-muted-foreground/60">
            <span className="size-1 rounded-full bg-current" />
            <span className="size-1 rounded-full bg-current" />
            <span className="size-1 rounded-full bg-current" />
          </div>
          <div className="flex size-10 items-center justify-center rounded-xl border border-border/60 bg-fg/[0.04]">
            <MonitorSmartphone className="size-5 text-foreground/80" />
          </div>
        </div>

        <h1 className="text-[22px] font-semibold tracking-tight text-foreground">Pair this device</h1>
        <p className="mt-1.5 text-[13.5px] leading-relaxed text-muted-foreground">
          On the computer running Mira, open <span className="text-foreground">Settings → Devices</span>{' '}
          and choose <span className="text-foreground">Pair a new device</span>. Enter the code it shows, or
          scan its QR code.
        </p>

        <form
          className="mt-7"
          onSubmit={(e) => {
            e.preventDefault();
            if (code.length === CODE_LEN && !busy) void submit(code);
          }}
        >
          <div
            className={cn('flex items-center justify-between gap-1.5 sm:gap-2', status === 'error' && 'animate-[mira-shake_0.4s_ease-in-out]')}
            onPaste={(e) => {
              e.preventDefault();
              fill(0, e.clipboardData.getData('text'));
            }}
          >
            {chars.map((c, i) => (
              <div key={i} className="contents">
                {i === CODE_LEN / 2 && <span className="w-2 shrink-0 text-center text-muted-foreground/50">–</span>}
                <input
                  ref={(el) => {
                    inputs.current[i] = el;
                  }}
                  value={c}
                  disabled={busy}
                  inputMode="text"
                  autoCapitalize="characters"
                  autoComplete="one-time-code"
                  spellCheck={false}
                  aria-label={`Character ${i + 1} of ${CODE_LEN}`}
                  // Whole value, not one char: autofill, one-time-code and
                  // dictation insert the full code into a single box.
                  onChange={(e) => fill(i, e.target.value)}
                  onKeyDown={(e) => onKey(i, e)}
                  onFocus={(e) => e.target.select()}
                  className={cn(
                    'h-12 w-full min-w-0 rounded-xl border bg-background/60 text-center font-mono text-[20px] font-semibold uppercase text-foreground caret-mira-blue outline-none transition-all sm:h-14',
                    'focus:border-mira-blue/70 focus:bg-background focus:shadow-[0_0_0_4px_rgb(var(--mira-blue)/0.15)]',
                    status === 'error' ? 'border-destructive/60' : c ? 'border-border' : 'border-border/50',
                    status === 'paired' && 'border-emerald-500/60 text-emerald-400',
                  )}
                />
              </div>
            ))}
          </div>

          <div className="mt-2 min-h-5 text-[12.5px]" aria-live="polite">
            {status === 'error' && <span className="text-destructive">{error}</span>}
            {status === 'pairing' && (
              <span className="inline-flex items-center gap-1.5 text-muted-foreground">
                <Loader2 className="size-3.5 animate-spin" /> Pairing…
              </span>
            )}
            {status === 'paired' && (
              <span className="inline-flex items-center gap-1.5 text-emerald-400">
                <Check className="size-3.5" /> Paired. Opening Mira…
              </span>
            )}
          </div>

          <label className="mt-5 block">
            <span className="text-[12px] font-medium text-muted-foreground">This device's name</span>
            <input
              value={name}
              disabled={busy}
              onChange={(e) => setName(e.target.value)}
              maxLength={60}
              className="mt-1.5 h-10 w-full rounded-xl border border-border/60 bg-background/60 px-3 text-[13.5px] text-foreground outline-none transition-colors focus:border-mira-blue/70"
            />
          </label>

          <button
            type="submit"
            disabled={code.length !== CODE_LEN || busy}
            className="mt-6 inline-flex h-11 w-full items-center justify-center gap-2 rounded-xl bg-foreground text-[14px] font-medium text-background transition-opacity disabled:opacity-40"
          >
            {status === 'paired' ? <Check className="size-4" /> : status === 'pairing' ? <Loader2 className="size-4 animate-spin" /> : null}
            {status === 'paired' ? 'Paired' : 'Pair device'}
          </button>
        </form>
      </div>
    </div>
  );
}
