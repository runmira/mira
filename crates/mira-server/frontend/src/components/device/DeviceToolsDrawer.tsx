/**
 * Device settings for the open device, in a side drawer. Each control shows
 * what the device reports back after the change, not what was asked for.
 */
import { useState, type ReactNode } from 'react';
import {
  Bell, Check, CircleAlert, LoaderCircle, MapPin, Moon, RotateCw, ShieldCheck, Sun, Type, Wifi, WifiOff, X,
} from 'lucide-react';
import { cn } from '@/lib/utils';
import { Button } from '../ui/button';
import { Tip } from '../ui/Tip';
import { PaneIconButton } from '../panes/paneUi';
import { TSwitch } from '../settings/SettingsFields';
import type { Device, DeviceAction } from './api';
import { useDeviceAction } from './queries';

/** iOS content size categories, smallest to largest, with short names. */
const IOS_TEXT_SIZES: [string, string][] = [
  ['extra-small', 'XS'], ['small', 'S'], ['medium', 'M'], ['large', 'Default'], ['extra-large', 'L'],
  ['extra-extra-large', 'XL'], ['extra-extra-extra-large', 'XXL'],
  ['accessibility-medium', 'AX1'], ['accessibility-large', 'AX2'], ['accessibility-extra-large', 'AX3'],
];
const ANDROID_TEXT_SIZES: [string, string][] = [['0.85', '0.85×'], ['1.0', 'Default'], ['1.15', '1.15×'], ['1.3', '1.3×'], ['1.5', '1.5×'], ['2.0', '2×']];
const IOS_SERVICES = ['photos', 'camera', 'microphone', 'location', 'contacts', 'calendar', 'reminders', 'motion', 'media-library', 'all'];
const ANDROID_SERVICES = ['CAMERA', 'RECORD_AUDIO', 'ACCESS_FINE_LOCATION', 'READ_CONTACTS', 'POST_NOTIFICATIONS'];
const PLACES = [
  { name: 'Cupertino', lat: 37.3349, lon: -122.009 },
  { name: 'London', lat: 51.5072, lon: -0.1276 },
  { name: 'Lagos', lat: 6.5244, lon: 3.3792 },
  { name: 'Tokyo', lat: 35.6762, lon: 139.6503 },
];
const PUSH_TEMPLATES = {
  Alert: { aps: { alert: { title: 'Hello from Mira', body: 'A test notification' }, sound: 'default' } },
  Badge: { aps: { badge: 7 } },
  Silent: { aps: { 'content-available': 1 } },
} as const;

const field = 'h-8 w-full min-w-0 rounded-lg border border-fg/[0.1] bg-shade/30 px-2.5 text-[12px] outline-none transition-[border-color,box-shadow] placeholder:text-muted-foreground/50 focus:border-mira-blue/70 focus:ring-2 focus:ring-mira-blue/15';
const chip = (on: boolean) => cn(
  'h-7 rounded-full border px-2.5 text-[11.5px] transition-colors',
  on ? 'border-mira-blue/50 bg-mira-blue/15 text-foreground' : 'border-fg/[0.1] text-muted-foreground hover:border-fg/[0.2] hover:text-foreground',
);

export function DeviceToolsDrawer({ device, onClose }: { device: Device; onClose: () => void }) {
  const ios = device.platform === 'ios';
  const sizes = ios ? IOS_TEXT_SIZES : ANDROID_TEXT_SIZES;
  const mutation = useDeviceAction(device.platform, device.id);
  const [last, setLast] = useState<string | null>(null);
  const [appearance, setAppearance] = useState<'light' | 'dark' | null>(null);
  const [size, setSize] = useState(ios ? 3 : 1);
  const [place, setPlace] = useState<string | null>(null);
  const [lat, setLat] = useState('');
  const [lon, setLon] = useState('');
  const [app, setApp] = useState('');
  const [service, setService] = useState(ios ? 'photos' : 'CAMERA');
  const [push, setPush] = useState(JSON.stringify(PUSH_TEMPLATES.Alert, null, 2));
  const [network, setNetwork] = useState({ wifi: true, data: true });

  const run = (name: string, action: DeviceAction) => {
    setLast(name);
    mutation.mutate(action, {
      onSuccess: (back) => {
        if (back.appearance === 'light' || back.appearance === 'dark') setAppearance(back.appearance);
        if (back.network && typeof back.network === 'object') setNetwork(back.network as { wifi: boolean; data: boolean });
      },
    });
  };
  const pending = (name: string) => mutation.isPending && last === name;
  const permission = ios ? service : `android.permission.${service}`;
  const applySize = () => run('text', { action: 'textSize', value: sizes[size][0] });

  return (
    <aside aria-label="Device tools" className="flex w-[288px] shrink-0 flex-col border-l border-border/60 bg-background">
      <div className="flex h-[37px] shrink-0 items-center gap-2 border-b border-border/60 px-3">
        <span className="text-[12.5px] font-medium">Tools</span>
        <span className="truncate text-[11px] text-muted-foreground">{device.name}</span>
        <span className="flex-1" />
        <PaneIconButton title="Close tools" tipAlign="end" onClick={onClose}><X className="size-3.5" /></PaneIconButton>
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-2.5 overflow-auto p-3">
        <Section icon={<Sun className="size-3.5" />} title="Appearance">
          <div role="radiogroup" aria-label="Appearance" className="grid grid-cols-2 gap-1 rounded-xl bg-fg/[0.05] p-1">
            {(['light', 'dark'] as const).map((v) => (
              <button
                key={v}
                type="button"
                role="radio"
                aria-checked={appearance === v}
                disabled={mutation.isPending}
                onClick={() => run(v, { action: 'appearance', value: v })}
                className={cn(
                  'flex h-8 items-center justify-center gap-1.5 rounded-lg text-[12px] capitalize transition-all',
                  appearance === v ? 'bg-background text-foreground shadow-sm ring-1 ring-fg/[0.08]' : 'text-muted-foreground hover:text-foreground',
                )}
              >
                {pending(v) ? <LoaderCircle className="size-3.5 animate-spin" /> : v === 'light' ? <Sun className="size-3.5" /> : <Moon className="size-3.5" />}
                {v}
              </button>
            ))}
          </div>
        </Section>

        <Section
          icon={<Type className="size-3.5" />}
          title="Text size"
          aside={<span className="rounded-full bg-fg/[0.06] px-2 py-0.5 text-[11px] tabular-nums text-muted-foreground">{pending('text') ? 'Applying…' : sizes[size][1]}</span>}
        >
          <div className="flex items-center gap-2.5">
            <span className="text-[11px] text-muted-foreground">A</span>
            <input
              type="range"
              aria-label="Text size"
              aria-valuetext={sizes[size][1]}
              min={0}
              max={sizes.length - 1}
              value={size}
              disabled={mutation.isPending}
              onChange={(e) => setSize(Number(e.target.value))}
              onPointerUp={applySize}
              onKeyUp={(e) => { if (e.key.startsWith('Arrow')) applySize(); }}
              className="h-1 flex-1 cursor-pointer accent-[rgb(var(--mira-blue))]"
            />
            <span className="text-[16px] font-medium text-muted-foreground">A</span>
          </div>
        </Section>

        <Section icon={<MapPin className="size-3.5" />} title="Location">
          <div className="flex flex-wrap gap-1">
            {PLACES.map((p) => (
              <button
                key={p.name}
                type="button"
                disabled={mutation.isPending}
                className={chip(place === p.name)}
                onClick={() => { setPlace(p.name); setLat(String(p.lat)); setLon(String(p.lon)); run('loc', { action: 'location', latitude: p.lat, longitude: p.lon }); }}
              >
                {p.name}
              </button>
            ))}
          </div>
          <div className="flex gap-1.5">
            <input aria-label="Latitude" placeholder="Latitude" inputMode="decimal" className={field} value={lat} onChange={(e) => { setLat(e.target.value); setPlace(null); }} />
            <input aria-label="Longitude" placeholder="Longitude" inputMode="decimal" className={field} value={lon} onChange={(e) => { setLon(e.target.value); setPlace(null); }} />
            <Tip label="Set this location" align="end">
              <Button
                size="sm"
                variant="outline"
                aria-label="Set this location"
                className="h-8 w-8 shrink-0 px-0"
                disabled={mutation.isPending || !lat || !lon || !Number.isFinite(+lat) || !Number.isFinite(+lon)}
                onClick={() => run('loc', { action: 'location', latitude: +lat, longitude: +lon })}
              >
                {pending('loc') ? <LoaderCircle className="size-3.5 animate-spin" /> : <Check className="size-3.5" />}
              </Button>
            </Tip>
          </div>
        </Section>

        <Section icon={<ShieldCheck className="size-3.5" />} title="Permissions">
          <input
            aria-label={ios ? 'Bundle id' : 'Package name'}
            placeholder={ios ? 'Bundle id, e.g. com.acme.app' : 'Package, e.g. com.acme.app'}
            className={field}
            value={app}
            onChange={(e) => setApp(e.target.value.trim())}
          />
          <div className="flex flex-wrap gap-1">
            {(ios ? IOS_SERVICES : ANDROID_SERVICES).map((s) => (
              <button key={s} type="button" className={chip(service === s)} onClick={() => setService(s)}>
                {s.toLowerCase().replace(/[_-]/g, ' ')}
              </button>
            ))}
          </div>
          <div className="grid grid-cols-2 gap-1.5">
            <Button size="sm" disabled={mutation.isPending || !app} onClick={() => run('grant', { action: 'permission', app, service: permission, grant: true })}>
              {pending('grant') && <LoaderCircle className="size-3.5 animate-spin" />}Grant
            </Button>
            <Button size="sm" variant="outline" disabled={mutation.isPending || !app} onClick={() => run('revoke', { action: 'permission', app, service: permission, grant: false })}>
              {pending('revoke') && <LoaderCircle className="size-3.5 animate-spin" />}Revoke
            </Button>
          </div>
        </Section>

        {ios ? (
          <Section icon={<Bell className="size-3.5" />} title="Push notification">
            <div className="flex gap-1">
              {(Object.keys(PUSH_TEMPLATES) as (keyof typeof PUSH_TEMPLATES)[]).map((k) => (
                <button key={k} type="button" className={chip(push === JSON.stringify(PUSH_TEMPLATES[k], null, 2))} onClick={() => setPush(JSON.stringify(PUSH_TEMPLATES[k], null, 2))}>
                  {k}
                </button>
              ))}
            </div>
            <textarea
              aria-label="Push payload"
              rows={5}
              spellCheck={false}
              className={cn(field, 'h-auto resize-none py-2 font-mono text-[11px] leading-relaxed', !validJson(push) && 'border-destructive/50')}
              value={push}
              onChange={(e) => setPush(e.target.value)}
            />
            <Tip label={app ? 'Deliver to the app above' : 'Enter the app’s bundle id under Permissions'} className="w-full">
              <Button size="sm" className="w-full" disabled={mutation.isPending || !app || !validJson(push)} onClick={() => run('push', { action: 'push', app, payload: JSON.parse(push) })}>
                {pending('push') ? <LoaderCircle className="size-3.5 animate-spin" /> : <Bell className="size-3.5" />}Send notification
              </Button>
            </Tip>
          </Section>
        ) : (
          <>
            <Section icon={<RotateCw className="size-3.5" />} title="Orientation">
              <div className="grid grid-cols-4 gap-1 rounded-xl bg-fg/[0.05] p-1">
                {([0, 90, 180, 270] as const).map((d) => (
                  <Tip key={d} label={`Turn to ${d}°`} className="w-full">
                    <button
                      type="button"
                      aria-label={`Turn to ${d}°`}
                      disabled={mutation.isPending}
                      onClick={() => run(`o${d}`, { action: 'orientation', degrees: d })}
                      className="flex h-8 w-full items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-background hover:text-foreground"
                    >
                      <span className="block h-3.5 w-2.5 rounded-[3px] border-[1.5px] border-current" style={{ transform: `rotate(${d}deg)` }} />
                    </button>
                  </Tip>
                ))}
              </div>
            </Section>
            <Section icon={network.wifi ? <Wifi className="size-3.5" /> : <WifiOff className="size-3.5" />} title="Network">
              <div className="flex items-center justify-between text-[12px]">
                <span>Wi‑Fi</span>
                <TSwitch checked={network.wifi} disabled={mutation.isPending} label="Wi‑Fi" onChange={(on) => run('wifi', { action: 'network', wifi: on })} />
              </div>
              <div className="flex items-center justify-between text-[12px]">
                <span>Mobile data</span>
                <TSwitch checked={network.data} disabled={mutation.isPending} label="Mobile data" onChange={(on) => run('data', { action: 'network', data: on })} />
              </div>
            </Section>
          </>
        )}
      </div>

      {(mutation.isSuccess || mutation.isError) && (
        <div
          key={mutation.submittedAt}
          role={mutation.isError ? 'alert' : 'status'}
          className={cn(
            'onb-in m-3 mt-0 flex items-start gap-2 rounded-xl border px-3 py-2 text-[11.5px]',
            mutation.isError ? 'border-destructive/30 bg-destructive/5 text-destructive' : 'border-fg/[0.08] bg-fg/[0.04] text-muted-foreground',
          )}
        >
          {mutation.isError ? <CircleAlert className="mt-px size-3.5 shrink-0" /> : <Check className="mt-px size-3.5 shrink-0 text-success" />}
          <span>{mutation.isError ? mutation.error.message : describe(mutation.data)}</span>
        </div>
      )}
    </aside>
  );
}

function Section({ icon, title, aside, children }: { icon: ReactNode; title: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2.5 rounded-2xl border border-fg/[0.08] bg-fg/[0.03] p-3">
      <header className="flex items-center gap-2">
        <span className="grid size-6 place-items-center rounded-lg bg-mira-blue/[0.12] text-mira-blue">{icon}</span>
        <h3 className="text-[12px] font-medium">{title}</h3>
        <span className="flex-1" />
        {aside}
      </header>
      {children}
    </section>
  );
}

function validJson(s: string) {
  try { JSON.parse(s); return true; } catch { return false; }
}

/** One line from what the device reported back. */
export function describe(back: Record<string, unknown>): string {
  if (typeof back.appearance === 'string') return `Appearance is ${back.appearance}.`;
  if (typeof back.textSize === 'string') return `Text size is ${back.textSize}.`;
  if (back.location && typeof back.location === 'object') {
    const l = back.location as { latitude: number; longitude: number };
    return `Location set to ${l.latitude}, ${l.longitude}.`;
  }
  if (back.permission && typeof back.permission === 'object') {
    const p = back.permission as { app: string; service: string; granted: boolean };
    return `${p.service} ${p.granted ? 'granted to' : 'revoked from'} ${p.app}.`;
  }
  if (back.pushed) return 'Notification sent.';
  if (typeof back.orientation === 'number') return `Turned to ${back.orientation}°.`;
  if (back.network && typeof back.network === 'object') {
    const n = back.network as { wifi: boolean; data: boolean };
    return `Wi‑Fi ${n.wifi ? 'on' : 'off'}, mobile data ${n.data ? 'on' : 'off'}.`;
  }
  return 'Done.';
}
