/**
 * First run of the Device pane: turn on the device hub (this is the
 * download), check what this computer can run, then choose whether agents
 * may use devices. Nothing downloads until the first switch is on.
 *
 * Styled like Mira's own onboarding: ambient light, segmented steps, each
 * step easing in. The phone at the top lights up as setup moves along.
 */
import { useState } from 'react';
import { ArrowLeft, ArrowRight, Check, CircleAlert, LoaderCircle, RefreshCw } from 'lucide-react';
import { cn } from '@/lib/utils';
import { Button } from '../ui/button';
import { TSwitch } from '../settings/SettingsFields';
import type { DeviceState, Platform } from './api';
import { InstallProgress } from './InstallProgress';
import { PlatformLogo } from './PlatformLogo';
import { useDeviceList, useDeviceSettings } from './queries';

const STEPS = [
  { title: 'Bring devices into Mira', eyebrow: 'Device hub' },
  { title: 'Check what this Mac can run', eyebrow: 'Simulators' },
  { title: 'Let agents test on devices?', eyebrow: 'Agent access' },
] as const;

const tile = 'rounded-2xl border border-fg/[0.08] bg-fg/[0.035]';

export function DeviceSetup({ state, onDone }: { state: DeviceState; onDone: () => void }) {
  const [step, setStep] = useState(0);
  const settings = useDeviceSettings();
  const hubReady = state.hub === 'ready';
  // Only on the support step: the check lists devices, which needs the hub.
  const list = useDeviceList(hubReady && step === 1);
  const busy = settings.isPending;
  const change = (body: { hubEnabled?: boolean; agentAccess?: boolean }) => settings.mutateAsync(body).catch(() => {});
  const canNext = step === 0 ? hubReady : step === 1 ? state.platforms.ios.available || state.platforms.android.available : true;
  const lit = step + (step === 0 ? (hubReady ? 1 : state.hubProgress ?? 0) : 1);

  return (
    <div className="relative flex h-full min-h-0 flex-col overflow-hidden">
      <Ambient />
      <div className="relative z-10 flex min-h-0 flex-1 flex-col overflow-auto">
        <div className="mx-auto flex w-full max-w-[440px] flex-col gap-6 px-6 pb-6 pt-8">
          <Phone lit={lit / STEPS.length} waiting={state.hub === 'installing' || state.hub === 'starting'} />

          <div className="flex gap-1.5" aria-label={`Step ${step + 1} of ${STEPS.length}`}>
            {STEPS.map((s, i) => (
              <button
                key={s.eyebrow}
                type="button"
                aria-label={s.eyebrow}
                disabled={i > step || busy}
                onClick={() => setStep(i)}
                className={cn(
                  'h-1 flex-1 rounded-full transition-colors duration-500',
                  i < step ? 'bg-mira-blue/70' : i === step ? 'bg-mira-blue' : 'bg-fg/[0.08]',
                )}
              />
            ))}
          </div>

          <div key={step} className="onb-in flex flex-col gap-5">
            <header className="flex flex-col gap-2">
              <span className="text-[11px] font-semibold uppercase tracking-[0.14em] text-mira-blue/90">
                {STEPS[step].eyebrow} · {step + 1} of {STEPS.length}
              </span>
              <h2 className="text-balance text-[21px] font-semibold leading-tight tracking-[-0.02em]">{STEPS[step].title}</h2>
              <p className="text-[13px] leading-relaxed text-muted-foreground">
                {step === 0 && 'Watch and use an iOS Simulator or Android Emulator right next to your chat. Agents see the same screen you do.'}
                {step === 1 && 'Simulators run on this computer. iOS needs Xcode; Android needs Android Studio’s SDK and a virtual device.'}
                {step === 2 && 'Agents can open a device, tap through your app and take screenshots to check their own mobile work.'}
              </p>
            </header>

            {step === 0 && (
              <div className={cn(tile, 'flex flex-col gap-4 p-4')}>
                <Toggle
                  title="Device hub"
                  description={`Downloaded once (v${state.versions.hub}), runs on this computer and only listens locally.`}
                  checked={state.hubEnabled}
                  disabled={busy || state.hub === 'installing' || state.hub === 'starting'}
                  onChange={(on) => void change({ hubEnabled: on })}
                />
                <HubStatus state={state} />
              </div>
            )}

            {step === 1 && (
              <div className="flex flex-col gap-2">
                <PlatformCard platform="ios" state={state} />
                <PlatformCard platform="android" state={state} />
                <div className="flex items-center justify-between gap-3 pt-1">
                  <span className="text-[11.5px] text-muted-foreground/80">
                    {list.error ? list.error.message : 'Installed something? Check again.'}
                  </span>
                  <Button size="sm" variant="outline" onClick={() => void list.refetch()} disabled={list.isFetching}>
                    {list.isFetching ? <LoaderCircle className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
                    Check
                  </Button>
                </div>
                {list.data?.notes
                  .filter((n) => !Object.values(state.platforms).some((p) => p.reason === n))
                  .map((n) => <p key={n} className="text-[12px] text-muted-foreground">{n}</p>)}
              </div>
            )}

            {step === 2 && (
              <div className={cn(tile, 'flex flex-col gap-4 p-4')}>
                <Toggle
                  title="Agent device access"
                  description={`Downloads agent-device (v${state.versions.agentDevice}). Applies to chats started after this.`}
                  checked={state.agentAccess}
                  disabled={busy || !state.hubEnabled}
                  onChange={(on) => void change({ agentAccess: on })}
                />
                {state.agentAccess && <AgentStatus state={state} />}
                {!state.agentAccess && (
                  <p className="text-[11.5px] text-muted-foreground/80">Leave this off to use devices yourself without giving agents access.</p>
                )}
              </div>
            )}

            {settings.error && (
              <p role="alert" className="rounded-xl border border-destructive/30 bg-destructive/5 px-3 py-2 text-[12px] text-destructive">
                {settings.error.message}
              </p>
            )}
          </div>

          <div className="flex items-center justify-between">
            <Button size="sm" variant="ghost" onClick={() => setStep(step - 1)} disabled={busy} className={cn(step === 0 && 'invisible')}>
              <ArrowLeft className="size-3.5" />Back
            </Button>
            {step < STEPS.length - 1 ? (
              <Button size="sm" onClick={() => setStep(step + 1)} disabled={!canNext || busy} className="group">
                Continue<ArrowRight className="size-3.5 transition-transform group-hover:translate-x-0.5" />
              </Button>
            ) : (
              <Button size="sm" onClick={onDone} disabled={busy || state.agentTools === 'installing'}>
                <Check className="size-3.5" />Open devices
              </Button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function Toggle({ title, description, checked, disabled, onChange }: {
  title: string; description: string; checked: boolean; disabled: boolean; onChange: (on: boolean) => void;
}) {
  return (
    <div className="flex items-start justify-between gap-4">
      <div>
        <div className="text-[13px] font-medium">{title}</div>
        <div className="mt-0.5 text-[12px] leading-relaxed text-muted-foreground">{description}</div>
      </div>
      <div className="shrink-0 pt-0.5">
        <TSwitch checked={checked} disabled={disabled} label={title} onChange={onChange} />
      </div>
    </div>
  );
}

function Line({ tone = 'ok', children }: { tone?: 'ok' | 'failed'; children: React.ReactNode }) {
  return (
    <p role={tone === 'failed' ? 'alert' : 'status'} className={cn('flex items-start gap-1.5 text-[12px]', tone === 'failed' ? 'text-destructive' : 'text-muted-foreground')}>
      {tone === 'failed' ? <CircleAlert className="mt-0.5 size-3.5 shrink-0" /> : <Check className="mt-0.5 size-3.5 shrink-0 text-success" />}
      <span>{children}</span>
    </p>
  );
}

function HubStatus({ state }: { state: DeviceState }) {
  if (!state.hubEnabled) return state.versions.hubInstalled ? <Line>Already downloaded. Turning it on just starts it.</Line> : null;
  switch (state.hub) {
    case 'installing': return <InstallProgress label={state.hubDetail ?? 'Installing device hub…'} value={state.hubProgress} />;
    case 'starting': return <InstallProgress label="Starting the device hub…" value={null} />;
    case 'ready': return <Line>Device hub is running{state.versions.node ? ` · Node ${state.versions.node}` : ''}.</Line>;
    case 'failed': return <Line tone="failed">{state.hubDetail ?? 'The device hub failed to start. Turn it off and on to retry.'}</Line>;
    default: return <InstallProgress label="Starting…" value={null} />;
  }
}

function AgentStatus({ state }: { state: DeviceState }) {
  switch (state.agentTools) {
    case 'installing': return <InstallProgress label={state.agentDetail ?? 'Installing agent tools…'} value={state.agentProgress} />;
    case 'ready': return <Line>Agent tools are ready. Start a new chat to use them.</Line>;
    case 'failed': return <Line tone="failed">{state.agentDetail ?? 'Agent tools failed to install.'}</Line>;
    default: return <InstallProgress label="Waiting for the device hub…" value={null} />;
  }
}

function PlatformCard({ platform, state }: { platform: Platform; state: DeviceState }) {
  const p = state.platforms[platform];
  return (
    <div className={cn(tile, 'flex items-start gap-3 p-3.5', p.available && 'border-mira-blue/25 bg-mira-blue/[0.05]')}>
      <span className={cn('grid size-8 shrink-0 place-items-center rounded-xl', p.available ? 'bg-mira-blue/15 text-mira-blue' : 'bg-fg/[0.06] text-muted-foreground')}>
        <PlatformLogo platform={platform} className={cn('size-4', platform === 'android' && p.available && 'text-[#3DDC84]')} />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5 text-[13px] font-medium">
          {platform === 'ios' ? 'iOS Simulator' : 'Android Emulator'}
          {p.available && <Check className="size-3.5 text-mira-blue" strokeWidth={3} />}
        </div>
        <div className="mt-0.5 text-[12px] leading-relaxed text-muted-foreground">{p.available ? 'Ready to go.' : p.reason}</div>
      </div>
    </div>
  );
}

/** A phone whose screen fills with light as setup moves along (0…1). */
function Phone({ lit, waiting }: { lit: number; waiting: boolean }) {
  return (
    <div className="relative mx-auto h-[118px] w-[64px]">
      <div
        className="absolute -inset-8 rounded-full bg-[radial-gradient(closest-side,rgba(122,162,247,0.22),transparent)] transition-opacity duration-700"
        style={{ opacity: 0.35 + Math.min(1, lit) * 0.65 }}
      />
      <div className="relative h-full w-full rounded-[16px] border border-fg/[0.14] bg-[#0b0c0f] p-[3px] shadow-[0_20px_40px_-18px_rgba(0,0,0,0.9)]">
        <div className="relative h-full w-full overflow-hidden rounded-[13px] bg-fg/[0.04]">
          <div
            className={cn(
              'absolute inset-x-0 bottom-0 bg-gradient-to-t from-mira-blue/70 via-mira-blue/35 to-[rgba(45,212,191,0.25)] transition-[height] duration-700 ease-out',
              waiting && 'device-breathe',
            )}
            style={{ height: `${Math.round(Math.min(1, lit) * 100)}%` }}
          />
          <span className="absolute left-1/2 top-[5px] h-[5px] w-[18px] -translate-x-1/2 rounded-full bg-black/80" />
        </div>
      </div>
    </div>
  );
}

/** Soft colour and a faint dot grid, like Mira's onboarding. */
function Ambient() {
  return (
    <div aria-hidden className="pointer-events-none absolute inset-0">
      <div className="absolute -left-32 -top-40 h-[420px] w-[420px] rounded-full bg-[radial-gradient(closest-side,rgba(122,162,247,0.14),transparent)]" />
      <div className="absolute -bottom-44 -right-32 h-[420px] w-[420px] rounded-full bg-[radial-gradient(closest-side,rgba(45,212,191,0.08),transparent)]" />
      <div
        className="absolute inset-0 opacity-50"
        style={{
          backgroundImage: 'radial-gradient(rgba(255,255,255,0.06) 1px, transparent 1px)',
          backgroundSize: '20px 20px',
          maskImage: 'radial-gradient(ellipse 70% 55% at 50% 30%, black, transparent)',
          WebkitMaskImage: 'radial-gradient(ellipse 70% 55% at 50% 30%, black, transparent)',
        }}
      />
    </div>
  );
}
