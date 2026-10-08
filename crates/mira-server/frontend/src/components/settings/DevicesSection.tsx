import { useCallback, useEffect, useRef, useState } from 'react';
import { Check, Copy, Globe, Laptop, Loader2, Monitor, Plus, RefreshCw, Smartphone, Tablet, X } from 'lucide-react';
import { cn } from '@/lib/utils';
import { QrCode } from '../pairing/QrCode';
import {
  listDevices,
  openPairingCode,
  revokeDevice,
  type DevicesView,
  type PairedDevice,
} from '../../lib/pairing';
import { getRemote, remoteAccessAvailable, turnOffRemote, turnOnRemote, type RemoteStatus } from '../../lib/remote';

/**
 * Settings → Devices: pair phones and other computers with this Mira, see
 * which are paired, and revoke them. This computer is always trusted; the
 * list is everything else. See crates/mira-server/src/pairing.rs.
 */
export function DevicesSection() {
  const [view, setView] = useState<DevicesView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pairing, setPairing] = useState<{ code: string; expiresAt: number; known: Set<string> } | null>(null);
  const [justPaired, setJustPaired] = useState<PairedDevice | null>(null);
  const [remote, setRemote] = useState<RemoteStatus | null>(null);
  const [remoteBusy, setRemoteBusy] = useState(false);
  const pairingStarting = useRef(false);
  const [pairingBusy, setPairingBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const v = await listDevices();
      setView(v);
      setError(null);
      return v;
    } catch (e) {
      setError((e as Error).message);
      return null;
    }
  }, []);

  useEffect(() => {
    void refresh();
    getRemote().then(setRemote).catch(() => {});
  }, [refresh]);

  // Follow the connector while it's coming up or recovering.
  const remoteSettling = remote?.status === 'starting' || remote?.status === 'reconnecting';
  useEffect(() => {
    if (!remoteSettling) return;
    const t = window.setInterval(() => getRemote().then(setRemote).catch(() => {}), 1500);
    return () => window.clearInterval(t);
  }, [remoteSettling]);

  async function setRemoteAccess(on: boolean): Promise<RemoteStatus | null> {
    if (!remote) return null;
    setRemoteBusy(true);
    setError(null);
    try {
      const next = on ? await turnOnRemote(remote) : await turnOffRemote(remote);
      setRemote(next);
      return next;
    } catch (e) {
      setError((e as Error).message);
      getRemote().then(setRemote).catch(() => {});
      return null;
    } finally {
      setRemoteBusy(false);
    }
  }

  // While a code is open, watch for the device that uses it.
  useEffect(() => {
    if (!pairing) return;
    const t = window.setInterval(async () => {
      const v = await refresh();
      const fresh = v?.devices.find((d) => !pairing.known.has(d.id));
      if (fresh) {
        setJustPaired(fresh);
        setPairing(null);
      } else if (Date.now() / 1000 > pairing.expiresAt) {
        setPairing(null);
      }
    }, 1500);
    return () => window.clearInterval(t);
  }, [pairing, refresh]);

  async function startPairing() {
    if (pairingStarting.current) return;
    pairingStarting.current = true;
    setPairingBusy(true);
    setJustPaired(null);
    try {
      // A phone needs remote access before it can reach this computer.
      if (remote && !remote.enabled && remote.supported && remoteAccessAvailable()) {
        if (!(await setRemoteAccess(true))) return;
      }
      const baseline = await refresh();
      if (!baseline) return;
      const { code, expires_at } = await openPairingCode();
      setPairing({ code, expiresAt: expires_at, known: new Set(baseline.devices.map((d) => d.id)) });
    } catch (e) {
      setError((e as Error).message);
    } finally {
      pairingStarting.current = false;
      setPairingBusy(false);
    }
  }

  async function revoke(id: string) {
    await revokeDevice(id).catch((e) => setError((e as Error).message));
    // Don't keep celebrating a device that's just been revoked.
    setJustPaired((d) => (d?.id === id ? null : d));
    void refresh();
  }

  const devices = view?.devices ?? [];

  return (
    <div className="space-y-6">
      <div className="text-[12.5px] leading-relaxed text-muted-foreground">
        Use Mira from your phone or another computer. Paired devices can do everything you can here; this
        computer is always allowed.
      </div>

      {error && (
        <div className="rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-[12px] text-destructive">
          {error}
        </div>
      )}

      {remote && (
        <RemoteAccessCard
          remote={remote}
          busy={remoteBusy}
          available={remoteAccessAvailable()}
          onToggle={(on) => void setRemoteAccess(on)}
        />
      )}

      {pairing ? (
        <PairingCard
          code={pairing.code}
          expiresAt={pairing.expiresAt}
          remote={remote}
          onCancel={() => setPairing(null)}
          onRenew={() => void startPairing()}
        />
      ) : justPaired ? (
        <PairedCard device={justPaired} onAnother={() => void startPairing()} onDone={() => setJustPaired(null)} />
      ) : (
        <button
          type="button"
          onClick={() => void startPairing()}
          disabled={remoteBusy || pairingBusy}
          className="group flex w-full items-center gap-4 rounded-2xl border border-dashed border-border/70 bg-fg/[0.015] p-5 text-left transition-all hover:-translate-y-0.5 hover:border-mira-blue/50 hover:bg-mira-blue/[0.04]"
        >
          <div className="flex size-11 shrink-0 items-center justify-center rounded-xl bg-mira-blue/[0.12] text-mira-blue transition-transform group-hover:scale-105">
            {remoteBusy || pairingBusy ? <Loader2 className="size-5 animate-spin" /> : <Plus className="size-5" />}
          </div>
          <div className="min-w-0">
            <div className="text-[14px] font-medium text-foreground">Pair a new device</div>
            <div className="mt-0.5 text-[12.5px] text-muted-foreground">
              Scan a QR code with your phone, or type a short code on any browser.
            </div>
          </div>
        </button>
      )}

      <div>
        <div className="mb-2.5 flex items-center justify-between px-1">
          <h3 className="text-[13px] font-medium text-muted-foreground">
            Paired devices{devices.length ? ` · ${devices.length}` : ''}
          </h3>
          <button
            type="button"
            onClick={() => void refresh()}
            className="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-fg/[0.05] hover:text-foreground"
            aria-label="Refresh"
          >
            <RefreshCw className="size-3.5" />
          </button>
        </div>
        {view === null ? (
          <div className="flex items-center gap-2 px-1 text-[12.5px] text-muted-foreground">
            <Loader2 className="size-3.5 animate-spin" /> Loading…
          </div>
        ) : devices.length === 0 ? (
          <div className="rounded-2xl border border-border/50 bg-fg/[0.015] px-5 py-6 text-center text-[12.5px] text-muted-foreground">
            No other devices yet.
          </div>
        ) : (
          <div className="grid gap-2.5 sm:grid-cols-2">
            {devices.map((d) => (
              <DeviceCard key={d.id} device={d} highlight={d.id === justPaired?.id} onRevoke={() => void revoke(d.id)} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function PairingCard({
  code,
  expiresAt,
  remote,
  onCancel,
  onRenew,
}: {
  code: string;
  expiresAt: number;
  remote: RemoteStatus | null;
  onCancel: () => void;
  onRenew: () => void;
}) {
  const now = useNow();
  const total = 600;
  const left = Math.max(0, Math.round(expiresAt - now / 1000));
  const expired = left === 0;
  const url = remote?.enabled && remote.hostname ? `https://${remote.hostname}/` : null;
  const qrValue = url ? `${url}?pair=${code.replace('-', '')}` : null;

  return (
    <div className="relative overflow-hidden rounded-2xl border border-border/60 bg-card/50 p-5 sm:p-6">
      <div aria-hidden className="pointer-events-none absolute -right-24 -top-24 size-64 rounded-full bg-mira-blue/10 blur-3xl" />
      <button
        type="button"
        onClick={onCancel}
        aria-label="Cancel pairing"
        className="absolute right-3 top-3 inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-fg/[0.06] hover:text-foreground"
      >
        <X className="size-4" />
      </button>

      <div className="relative flex flex-col items-center gap-6 sm:flex-row sm:items-center">
        {qrValue && !expired ? (
          <QrCode value={qrValue} />
        ) : (
          <div className="flex size-[184px] shrink-0 items-center justify-center rounded-2xl border border-dashed border-border/70 px-6 text-center text-[12px] text-muted-foreground">
            {expired ? 'Code expired' : 'Turn on remote access to scan with your phone'}
          </div>
        )}

        <div className="min-w-0 flex-1 text-center sm:text-left">
          <div className="text-[12px] font-medium uppercase tracking-wider text-muted-foreground">Pairing code</div>
          <div
            className={cn(
              'mt-1.5 select-all font-mono text-[34px] font-semibold tracking-[0.12em] text-foreground',
              expired && 'text-muted-foreground/50 line-through',
            )}
          >
            {code}
          </div>
          <div className="mt-2 flex items-center justify-center gap-2 text-[12.5px] text-muted-foreground sm:justify-start">
            <Countdown fraction={left / total} />
            {expired ? (
              <button type="button" onClick={onRenew} className="text-mira-blue hover:underline">
                Get a new code
              </button>
            ) : (
              <span>
                Expires in {Math.floor(left / 60)}:{String(left % 60).padStart(2, '0')} · works once
              </span>
            )}
          </div>

          <ol className="mt-5 space-y-1.5 text-[12.5px] leading-relaxed text-muted-foreground">
            {url ? (
              <>
                <li>
                  <span className="text-foreground">Phone:</span> point the camera at the QR code.
                </li>
                <li>
                  <span className="text-foreground">Anything else:</span> open{' '}
                  <CopyText text={url} /> and enter the code.
                </li>
              </>
            ) : (
              <li>Open this Mira on the other device and enter the code.</li>
            )}
          </ol>
        </div>
      </div>

    </div>
  );
}

function RemoteAccessCard({
  remote,
  busy,
  available,
  onToggle,
}: {
  remote: RemoteStatus;
  busy: boolean;
  available: boolean;
  onToggle: (on: boolean) => void;
}) {
  const on = remote.enabled;
  const url = remote.hostname ? `https://${remote.hostname}` : null;
  const blocked = !remote.supported || !available;
  return (
    <div className="flex items-center gap-4 rounded-2xl border border-border/60 bg-fg/[0.02] p-4 sm:p-5">
      <div
        className={cn(
          'relative flex size-11 shrink-0 items-center justify-center rounded-xl transition-colors',
          on ? 'bg-emerald-500/15 text-emerald-400' : 'bg-fg/[0.05] text-muted-foreground',
        )}
      >
        <Globe className="size-5" />
        {on && remote.status === 'connected' && (
          <span className="absolute -right-0.5 -top-0.5 size-2.5 animate-pulse rounded-full bg-emerald-400 ring-2 ring-background" />
        )}
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-[14px] font-medium text-foreground">Reach Mira from anywhere</div>
        <div className="mt-0.5 text-[12.5px] text-muted-foreground">
          {!remote.supported ? (
            'Not available on this platform yet.'
          ) : !available ? (
            'Sign in to Mira to turn this on.'
          ) : !on ? (
            'Use Mira from your phone on any network. Nothing to install or configure.'
          ) : remote.status === 'connected' && url ? (
            <span className="inline-flex flex-wrap items-center gap-1.5">
              Connected at <CopyText text={url} />
            </span>
          ) : remote.status === 'reconnecting' ? (
            <span className="text-amber-400/90">Reconnecting…{remote.error ? ` ${remote.error}` : ''}</span>
          ) : (
            <span className="inline-flex items-center gap-1.5">
              <Loader2 className="size-3 animate-spin" /> Connecting…
            </span>
          )}
        </div>
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={on}
        aria-label="Reach Mira from anywhere"
        disabled={busy || blocked}
        onClick={() => onToggle(!on)}
        className={cn(
          'relative h-6 w-11 shrink-0 rounded-full transition-colors disabled:opacity-50',
          on ? 'bg-emerald-500' : 'bg-fg/[0.15]',
        )}
      >
        <span
          className={cn(
            'absolute left-0.5 top-0.5 flex size-5 items-center justify-center rounded-full bg-white shadow transition-transform',
            on && 'translate-x-5',
          )}
        >
          {busy && <Loader2 className="size-3 animate-spin text-neutral-500" />}
        </span>
      </button>
    </div>
  );
}

function PairedCard({ device, onAnother, onDone }: { device: PairedDevice; onAnother: () => void; onDone: () => void }) {
  return (
    <div className="flex items-center gap-4 rounded-2xl border border-emerald-500/30 bg-emerald-500/[0.06] p-5">
      <div className="flex size-11 shrink-0 animate-[mira-pop_0.4s_ease-out] items-center justify-center rounded-full bg-emerald-500/15 text-emerald-400">
        <Check className="size-5" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-[14px] font-medium text-foreground">{device.name} is paired</div>
        <div className="mt-0.5 text-[12.5px] text-muted-foreground">It can use Mira until you revoke it below.</div>
      </div>
      <div className="flex shrink-0 gap-2">
        <button type="button" onClick={onAnother} className="rounded-lg px-3 py-1.5 text-[12.5px] text-muted-foreground hover:bg-fg/[0.05] hover:text-foreground">
          Pair another
        </button>
        <button type="button" onClick={onDone} className="rounded-lg bg-foreground px-3 py-1.5 text-[12.5px] font-medium text-background">
          Done
        </button>
      </div>
    </div>
  );
}

function DeviceCard({ device, highlight, onRevoke }: { device: PairedDevice; highlight: boolean; onRevoke: () => void }) {
  const [confirm, setConfirm] = useState(false);
  const now = useNow(30_000);
  const Icon = deviceIcon(device.name);
  const active = device.last_seen_at !== null && now / 1000 - device.last_seen_at < 120;

  useEffect(() => {
    if (!confirm) return;
    const t = window.setTimeout(() => setConfirm(false), 4000);
    return () => window.clearTimeout(t);
  }, [confirm]);

  return (
    <div
      className={cn(
        'group flex items-center gap-3 rounded-2xl border bg-fg/[0.02] p-3.5 transition-colors',
        highlight ? 'border-emerald-500/40' : 'border-border/60',
      )}
    >
      <div className="relative flex size-10 shrink-0 items-center justify-center rounded-xl bg-fg/[0.05] text-foreground/80">
        <Icon className="size-[18px]" />
        {active && (
          <span className="absolute -right-0.5 -top-0.5 size-2.5 rounded-full bg-emerald-400 ring-2 ring-background" title="Active now" />
        )}
      </div>
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13.5px] font-medium text-foreground">{device.name}</div>
        <div className="truncate text-[12px] text-muted-foreground">
          {active ? 'Active now' : device.last_seen_at ? `Seen ${ago(now / 1000 - device.last_seen_at)}` : 'Never used'} ·
          paired {new Date(device.created_at * 1000).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}
        </div>
      </div>
      <button
        type="button"
        onClick={() => (confirm ? onRevoke() : setConfirm(true))}
        className={cn(
          'shrink-0 rounded-lg px-2.5 py-1.5 text-[12px] transition-all',
          confirm
            ? 'bg-destructive/15 font-medium text-destructive'
            : 'text-muted-foreground opacity-70 hover:bg-fg/[0.06] hover:text-foreground group-hover:opacity-100',
        )}
      >
        {confirm ? 'Revoke?' : 'Revoke'}
      </button>
    </div>
  );
}

/** A ring that drains as the code nears expiry. */
function Countdown({ fraction }: { fraction: number }) {
  const r = 7;
  const c = 2 * Math.PI * r;
  return (
    <svg width={18} height={18} viewBox="0 0 18 18" className="-rotate-90" aria-hidden>
      <circle cx={9} cy={9} r={r} fill="none" stroke="currentColor" strokeOpacity={0.2} strokeWidth={2} />
      <circle
        cx={9}
        cy={9}
        r={r}
        fill="none"
        stroke="rgb(var(--mira-blue))"
        strokeWidth={2}
        strokeLinecap="round"
        strokeDasharray={c}
        strokeDashoffset={c * (1 - Math.max(0, Math.min(1, fraction)))}
        className="transition-[stroke-dashoffset] duration-1000 ease-linear"
      />
    </svg>
  );
}

function CopyText({ text, mono }: { text: string; mono?: boolean }) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<number>(0);
  return (
    <button
      type="button"
      onClick={() => {
        void navigator.clipboard?.writeText(text);
        setCopied(true);
        window.clearTimeout(timer.current);
        timer.current = window.setTimeout(() => setCopied(false), 1400);
      }}
      className={cn(
        'inline-flex max-w-full items-center gap-1.5 rounded-md border border-border/60 bg-background/60 px-1.5 py-0.5 align-middle text-foreground transition-colors hover:border-border',
        mono && 'font-mono text-[11.5px]',
      )}
      title="Copy"
    >
      <span className="truncate">{text}</span>
      {copied ? <Check className="size-3 shrink-0 text-emerald-400" /> : <Copy className="size-3 shrink-0 opacity-60" />}
    </button>
  );
}

function useNow(every = 1000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), every);
    return () => window.clearInterval(t);
  }, [every]);
  return now;
}

function deviceIcon(name: string) {
  if (/iPhone|Android/i.test(name)) return Smartphone;
  if (/iPad|Tablet/i.test(name)) return Tablet;
  if (/Mac|Windows|Linux/i.test(name)) return Laptop;
  return Monitor;
}

function ago(seconds: number): string {
  if (seconds < 3600) return `${Math.max(1, Math.round(seconds / 60))}m ago`;
  if (seconds < 86400) return `${Math.round(seconds / 3600)}h ago`;
  return `${Math.round(seconds / 86400)}d ago`;
}
