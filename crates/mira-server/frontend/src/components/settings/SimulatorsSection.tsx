/** Settings → Simulators: the device hub and agents' access to devices. Saves as you change it. */
import { Bot, Check, CircleAlert, LoaderCircle, MonitorSmartphone, Package } from 'lucide-react';
import { cn } from '@/lib/utils';
import { TNote, TRow, TSection, TSwitch } from './SettingsFields';
import type { DeviceState } from '../device/api';
import { useDeviceSettings, useDeviceState } from '../device/queries';
import { InstallProgress } from '../device/InstallProgress';

export function SimulatorsSection() {
  const { data: state, error } = useDeviceState();
  const settings = useDeviceSettings();
  const pending = settings.isPending;
  const saveError = settings.error?.message ?? '';
  const change = (body: { hubEnabled?: boolean; agentAccess?: boolean }) => settings.mutateAsync(body).catch(() => {});

  if (!state) {
    return <div role={error ? 'alert' : 'status'} className="rounded-xl border border-border/60 bg-card/40 p-4 text-[12.5px] text-muted-foreground">{error?.message || 'Loading…'}</div>;
  }
  const busy = pending || state.hub === 'installing' || state.hub === 'starting';
  return (
    <div className="flex flex-col gap-5">
      {saveError && <p role="alert" className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-[12px] text-destructive">{saveError}</p>}
      <TSection icon={<MonitorSmartphone className="size-3.5" />} title="Device hub">
        <TRow
          title="Device hub"
          description="Streams iOS Simulators and Android Emulators into the Device pane. Turning it off stops Mira’s helpers; running devices stay on."
          status={<HubStatus state={state} />}
          control={<TSwitch checked={state.hubEnabled} disabled={busy} label="Device hub" onChange={(on) => void change({ hubEnabled: on })} />}
        >
          {state.hub === 'installing' && <InstallProgress className="mt-3" label={state.hubDetail ?? 'Installing device hub…'} value={state.hubProgress} />}
        </TRow>
      </TSection>
      <TSection icon={<Bot className="size-3.5" />} title="Agents">
        <TRow
          title="Agent device access"
          description="Let agents open, tap through and screenshot devices. Applies to chats started after the change."
          status={state.agentAccess ? <Status pending={state.agentTools === 'installing'} failed={state.agentTools === 'failed'}
            text={state.agentTools === 'installing' ? 'Installing agent tools…' : state.agentTools === 'failed' ? state.agentDetail ?? 'Install failed' : state.agentTools === 'ready' ? 'Ready' : 'Waiting for the hub'} /> : undefined}
          control={<TSwitch checked={state.agentAccess} disabled={pending || !state.hubEnabled} label="Agent device access" onChange={(on) => void change({ agentAccess: on })} />}
        >
          {state.agentTools === 'installing' && <InstallProgress className="mt-3" label={state.agentDetail ?? 'Installing agent tools…'} value={state.agentProgress} />}
        </TRow>
      </TSection>
      <TSection icon={<Package className="size-3.5" />} title="Tool versions">
        <TRow title="Device hub" description={`Pinned ${state.versions.hub}`} status={<Installed on={state.versions.hubInstalled} />} />
        <TRow title="agent-device" description={`Pinned ${state.versions.agentDevice}`} status={<Installed on={state.versions.agentDeviceInstalled} />} />
        <TRow title="Node" description={state.versions.node ?? 'Found or downloaded when the hub first starts.'} />
      </TSection>
      <TNote>Tools are downloaded only when you turn them on, into ~/.mira/tools. New pinned versions install the next time they’re needed.</TNote>
    </div>
  );
}

function Status({ pending, failed, text }: { pending?: boolean; failed?: boolean; text: string }) {
  return (
    <span className={cn('inline-flex items-center gap-1 text-[11.5px]', failed ? 'text-destructive' : 'text-muted-foreground')}>
      {pending ? <LoaderCircle className="size-3 animate-spin" /> : failed ? <CircleAlert className="size-3" /> : <Check className="size-3 text-success" />}
      {text}
    </span>
  );
}

function HubStatus({ state }: { state: DeviceState }) {
  switch (state.hub) {
    case 'off': return null;
    case 'installing': return <Status pending text="Installing…" />;
    case 'starting': return <Status pending text="Starting…" />;
    case 'ready': return <Status text="Running" />;
    case 'failed': return <Status failed text={state.hubDetail ?? 'Failed'} />;
  }
}

function Installed({ on }: { on: boolean }) {
  return <span className="text-[11.5px] text-muted-foreground">{on ? 'Installed' : 'Not downloaded'}</span>;
}
