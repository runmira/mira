/**
 * The Device pane: a live iOS Simulator or Android Emulator you can watch
 * and use, the same devices agents drive. One tab per device; only the
 * visible one streams. First use goes through setup, which is where the
 * device hub is downloaded.
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import {
  ArrowLeft, Camera, Circle, CircleAlert, LayoutGrid, LoaderCircle, Plus, Power, RefreshCw, RotateCw,
  Settings2, Smartphone, SlidersHorizontal, X,
} from 'lucide-react';
import { cn } from '@/lib/utils';
import { Button } from '../ui/button';
import { Tip } from '../ui/Tip';
import { PaneBar, PaneEmpty, PaneIconButton } from '../panes/paneUi';
import { screenshotDevice, type Device, type DeviceList, type DeviceState } from './api';
import { useDeviceList, useDevicePower, useDeviceSettings, useDeviceState } from './queries';
import { DeviceSetup } from './DeviceSetup';
import { InstallProgress } from './InstallProgress';
import { onDeviceFocus, takePendingFocus } from './focus';
import { PlatformLogo } from './PlatformLogo';
import { DeviceStreamView, type DeviceStreamHandle } from './DeviceStreamView';
import { DeviceToolsDrawer } from './DeviceToolsDrawer';

const SETUP_KEY = 'mira.deviceSetupDone';
const TABS_KEY = 'mira.deviceTabs';

const read = <T,>(key: string, fallback: T): T => {
  try { return (JSON.parse(localStorage.getItem(key) ?? 'null') as T) ?? fallback; } catch { return fallback; }
};
const write = (key: string, value: unknown) => {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* storage unavailable */ }
};

export function DevicePane() {
  const { data: state, error } = useDeviceState();
  const [setupDone, setSetupDone] = useState(() => read(SETUP_KEY, false));
  const [settingUp, setSettingUp] = useState(false);

  if (!state) {
    return error
      ? <PaneEmpty icon={<CircleAlert className="size-4" />} title="Devices aren’t available">{error.message}</PaneEmpty>
      : <PaneEmpty icon={<LoaderCircle className="size-4 animate-spin" />} title="Loading devices…" />;
  }
  if (settingUp || !setupDone || !state.hubEnabled) {
    return <DeviceSetup state={state} onDone={() => { write(SETUP_KEY, true); setSetupDone(true); setSettingUp(false); }} />;
  }
  if (state.hub !== 'ready') return <HubNotReady state={state} />;
  return <Devices state={state} onSetup={() => setSettingUp(true)} />;
}

function HubNotReady({ state }: { state: DeviceState }) {
  const settings = useDeviceSettings();
  const busy = settings.isPending;
  if (state.hub === 'failed') {
    const retry = async () => {
      await settings.mutateAsync({ hubEnabled: false });
      await settings.mutateAsync({ hubEnabled: true });
    };
    return (
      <PaneEmpty icon={<CircleAlert className="size-4" />} title="The device hub stopped"
        action={<Button size="sm" variant="outline" onClick={() => void retry()} disabled={busy}>{busy ? <LoaderCircle className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}Retry</Button>}>
        {state.hubDetail}
      </PaneEmpty>
    );
  }
  return (
    <PaneEmpty icon={<Smartphone className="size-4" />} title={state.hub === 'installing' ? 'Setting up devices' : 'Starting the device hub…'}>
      <InstallProgress
        className="mt-2 w-[240px] text-left"
        label={state.hubDetail ?? (state.hub === 'installing' ? 'Installing device hub…' : 'Starting…')}
        value={state.hub === 'installing' ? state.hubProgress : null}
      />
    </PaneEmpty>
  );
}

type Tab = { platform: Device['platform']; id: string; name: string };

function Devices({ state, onSetup }: { state: DeviceState; onSetup: () => void }) {
  const listQuery = useDeviceList(true);
  const list = listQuery.data ?? null;
  const listError = listQuery.error?.message ?? '';
  const { boot, shutdown } = useDevicePower();
  const [tabs, setTabState] = useState<Tab[]>(() => read<Tab[]>(TABS_KEY, []).filter((t) => t && typeof t.id === 'string'));
  const setTabs = (update: (prev: Tab[]) => Tab[]) =>
    setTabState((prev) => { const next = update(prev); write(TABS_KEY, next); return next; });
  const [active, setActive] = useState<string | null>(() => tabs.at(-1)?.id ?? null);
  const [picking, setPicking] = useState(tabs.length === 0);
  const booting = boot.isPending ? boot.variables?.id ?? null : null;
  const [bootError, setBootError] = useState('');
  const [tools, setTools] = useState(false);
  const [inputReady, setInputReady] = useState(false);
  const view = useRef<DeviceStreamHandle | null>(null);

  const refresh = () => void listQuery.refetch();

  // An agent opened a device: show it on its own tab once the list has it.
  const [wanted, setWanted] = useState<string | null>(takePendingFocus);
  useEffect(() => onDeviceFocus((device) => {
    takePendingFocus();
    setWanted(device);
    void listQuery.refetch();
  }), [listQuery.refetch]);
  const found = wanted
    ? list?.devices.find((d) => d.id === wanted) ??
      list?.devices.find((d) => d.booted && d.name.toLowerCase() === wanted.toLowerCase())
    : undefined;
  if (wanted && found) {
    setWanted(null);
    setTabs((prev) => [...prev.filter((t) => t.id !== found.id), { platform: found.platform, id: found.id, name: found.name }]);
    setActive(found.id);
    setPicking(false);
  }

  const byId = useMemo(() => new Map((list?.devices ?? []).map((d) => [d.id, d])), [list]);
  const current = active ? byId.get(active) ?? null : null;
  const activeTab = tabs.find((t) => t.id === active) ?? null;

  const open = async (d: Device) => {
    setBootError('');
    try {
      // Booting a running iOS device just attaches its stream.
      const { id } = await boot.mutateAsync({ platform: d.platform, id: d.id });
      setTabs((prev) => [...prev.filter((t) => t.id !== d.id && t.id !== id), { platform: d.platform, id, name: d.name }]);
      setActive(id);
      setPicking(false);
    } catch (e) {
      setBootError((e as Error).message);
    }
  };
  const close = (id: string) => {
    setTabs((prev) => {
      const next = prev.filter((t) => t.id !== id);
      if (active === id) setActive(next.at(-1)?.id ?? null);
      if (!next.length) setPicking(true);
      return next;
    });
  };
  const powerOff = async () => {
    if (!current) return;
    await shutdown.mutateAsync({ platform: current.platform, id: current.id }).catch((e: Error) => setBootError(e.message));
    close(current.id);
  };
  const screenshot = async () => {
    if (!current) return;
    const blob = await screenshotDevice(current.platform, current.id).catch((e: Error) => { setBootError(e.message); return null; });
    if (!blob) return;
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = `${current.name.replace(/[^\w-]+/g, '-')}-${Date.now()}.png`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 1000);
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div role="tablist" aria-label="Devices" className="flex min-h-[33px] shrink-0 items-center gap-0.5 overflow-x-auto border-b border-border/60 px-1.5">
        {tabs.map((t) => (
          <div key={t.id} role="tab" aria-selected={!picking && t.id === active}
            className={cn('group flex h-6 shrink-0 items-center gap-1 rounded-md pl-2 pr-1 text-[11.5px]', !picking && t.id === active ? 'bg-fg/[0.08] text-foreground' : 'text-muted-foreground hover:bg-fg/[0.04]')}>
            <button type="button" className="flex items-center gap-1" onClick={() => { setActive(t.id); setPicking(false); }}>
              <Circle className={cn('size-1.5 fill-current', byId.get(t.id)?.booted ? 'text-success' : 'text-muted-foreground/50')} />
              {t.name}
            </button>
            <Tip label="Stop watching" hint="The device keeps running" side="bottom">
              <button type="button" aria-label={`Stop watching ${t.name}`} onClick={() => close(t.id)}
                className="rounded p-0.5 opacity-60 hover:bg-fg/[0.08] hover:opacity-100"><X className="size-3" /></button>
            </Tip>
          </div>
        ))}
        <PaneIconButton title="Open a device" active={picking} onClick={() => setPicking(true)}><Plus className="size-3.5" /></PaneIconButton>
        <span className="flex-1" />
        <PaneIconButton title="Device setup" tipAlign="end" onClick={onSetup}><Settings2 className="size-3.5" /></PaneIconButton>
      </div>

      {picking || !activeTab ? (
        <Picker list={list} error={listError || bootError} booting={booting} onOpen={(d) => void open(d)} onRefresh={refresh} />
      ) : (
        <>
          <PaneBar>
            <span className="min-w-0 truncate text-[12px] font-medium">{activeTab.name}</span>
            <span className="truncate text-[11px] text-muted-foreground">{current?.os}</span>
            {current && !inputReady && current.booted && <span className="text-[11px] text-muted-foreground/80">· view only</span>}
            <span className="flex-1" />
            {activeTab.platform === 'android' ? (
              <>
                <PaneIconButton title="Back" onClick={() => view.current?.button('back')}><ArrowLeft className="size-3.5" /></PaneIconButton>
                <PaneIconButton title="Home" onClick={() => view.current?.button('home')}><Circle className="size-3.5" /></PaneIconButton>
                <PaneIconButton title="Recent apps" onClick={() => view.current?.button('recents')}><LayoutGrid className="size-3.5" /></PaneIconButton>
              </>
            ) : (
              <>
                <PaneIconButton title="Home" onClick={() => view.current?.button('home')}><Circle className="size-3.5" /></PaneIconButton>
                <PaneIconButton title="Rotate" onClick={() => view.current?.rotate()}><RotateCw className="size-3.5" /></PaneIconButton>
              </>
            )}
            <PaneIconButton title="Save a screenshot" onClick={() => void screenshot()} disabled={!current?.booted}><Camera className="size-3.5" /></PaneIconButton>
            <PaneIconButton title="Tools" tipAlign="end" active={tools} onClick={() => setTools((v) => !v)} disabled={!current?.booted}><SlidersHorizontal className="size-3.5" /></PaneIconButton>
            <PaneIconButton title="Power off" tipAlign="end" tone="danger" onClick={() => void powerOff()} disabled={!current?.booted}><Power className="size-3.5" /></PaneIconButton>
          </PaneBar>
          {bootError && <p role="alert" className="border-b border-destructive/20 bg-destructive/5 px-3 py-1.5 text-[12px] text-destructive">{bootError}</p>}
          <div className="flex min-h-0 flex-1">
            <div className="min-h-0 min-w-0 flex-1 bg-fg/[0.025]">
              {!list ? (
                <PaneEmpty icon={<LoaderCircle className="size-4 animate-spin" />} title="Loading devices…" />
              ) : !current ? (
                <PaneEmpty icon={<Smartphone className="size-4" />} title={`${activeTab.name} isn’t available`}
                  action={<Button size="sm" variant="outline" onClick={() => close(activeTab.id)}>Close tab</Button>}>It may have been deleted.</PaneEmpty>
              ) : !current.booted || !current.stream ? (
                <PaneEmpty icon={<Power className="size-4" />} title={`${current.name} is off`}
                  action={<Button size="sm" onClick={() => void open(current)} disabled={!!booting}>{booting ? <LoaderCircle className="size-3.5 animate-spin" /> : null}Start</Button>} />
              ) : (
                <DeviceStreamView key={current.id} ref={view} device={current} onInput={setInputReady} />
              )}
            </div>
            {tools && current?.booted && <DeviceToolsDrawer device={current} onClose={() => setTools(false)} />}
          </div>
        </>
      )}
      {!state.agentAccess && (
        <p className="shrink-0 border-t border-border/60 px-3 py-1.5 text-[11px] text-muted-foreground">
          Agents can’t use devices. <button type="button" className="underline-offset-2 hover:underline" onClick={onSetup}>Change in setup</button>
        </p>
      )}
    </div>
  );
}

function Picker({ list, error, booting, onOpen, onRefresh }: {
  list: DeviceList | null; error: string; booting: string | null; onOpen: (d: Device) => void; onRefresh: () => void;
}) {
  if (!list) {
    return error
      ? <PaneEmpty icon={<CircleAlert className="size-4" />} title="Couldn’t list devices" action={<Button size="sm" variant="outline" onClick={onRefresh}><RefreshCw className="size-3.5" />Retry</Button>}>{error}</PaneEmpty>
      : <PaneEmpty icon={<LoaderCircle className="size-4 animate-spin" />} title="Looking for devices…" />;
  }
  const running = list.devices.filter((d) => d.booted);
  const stopped = list.devices.filter((d) => !d.booted);
  return (
    <div className="min-h-0 flex-1 overflow-auto p-3">
      {error && <p role="alert" className="mb-3 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-[12px] text-destructive">{error}</p>}
      {!list.devices.length && (
        <PaneEmpty icon={<Smartphone className="size-4" />} title="No devices yet" action={<Button size="sm" variant="outline" onClick={onRefresh}><RefreshCw className="size-3.5" />Check again</Button>}>
          {list.notes.join(' ') || 'Create a simulator in Xcode or a virtual device in Android Studio.'}
        </PaneEmpty>
      )}
      {[['Running', running], ['Off', stopped]].map(([title, items]) => (items as Device[]).length > 0 && (
        <section key={title as string} className="mb-4">
          <h3 className="mb-1.5 px-1 text-[10.5px] font-medium uppercase tracking-wide text-muted-foreground/80">{title as string}</h3>
          <ul className="divide-y divide-border/50 rounded-xl border border-border/60 bg-card/40">
            {(items as Device[]).map((d) => (
              <li key={d.id} className="flex items-center gap-2.5 px-3 py-2">
                <span className="grid size-7 shrink-0 place-items-center rounded-lg bg-fg/[0.06] text-foreground/80">
                  <PlatformLogo platform={d.platform} className="size-3.5" />
                </span>
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[12.5px] font-medium">{d.name}</div>
                  <div className="truncate text-[11px] text-muted-foreground">{d.os}</div>
                </div>
                <Button size="sm" variant={d.booted ? 'outline' : 'default'} disabled={!!booting} onClick={() => onOpen(d)}>
                  {booting === d.id && <LoaderCircle className="size-3.5 animate-spin" />}
                  {d.booted ? 'Watch' : booting === d.id ? 'Starting…' : 'Start'}
                </Button>
              </li>
            ))}
          </ul>
        </section>
      ))}
      {list.devices.length > 0 && list.notes.length > 0 && (
        <p className="px-1 text-[11.5px] leading-relaxed text-muted-foreground">{list.notes.join(' ')}</p>
      )}
    </div>
  );
}
