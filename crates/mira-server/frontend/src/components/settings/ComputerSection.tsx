import { useState } from 'react';
import { Check, LoaderCircle, Monitor, MousePointer2, Plus, RefreshCw, ShieldCheck, Unplug } from 'lucide-react';
import { Button } from '../ui/button';
import { Select } from '../ui/select';
import { cn } from '@/lib/utils';
import { TNote, TRow, TSection, TSwitch } from './SettingsFields';
import { ComputerIllustration } from './computer/ComputerIllustration';
import { PairComputerDialog } from './computer/PairComputerDialog';
import { useComputerSettings } from './computer/useComputerSettings';
import type { Delivery } from './computer/api';

const DELIVERY_DESCRIPTION: Record<Delivery, string> = {
  background: 'Keep your pointer and focus. Pause if an action needs foreground access.',
  auto: 'Prefer the background. Your pointer and focus may move when needed.',
  foreground: 'Use the visible pointer and keyboard on the selected computer.',
};

export function ComputerSection({ preview = false }: { preview?: boolean }) {
  const { view, error, pending, identity, windows, connectionError, checking, refresh, check, change, paired, remove } = useComputerSettings(preview);
  const [pairing, setPairing] = useState<'connect' | 'share' | null>(null);
  const [notice, setNotice] = useState('');
  const disabled = pending || !view?.can_change;
  const windowKey = view?.window ? `${view.window.id}:${view.window.epoch}` : '';
  const windowOptions = windows.map((window) => ({ value: `${window.id}:${window.epoch}`, label: window.title || window.app, hint: window.app }));
  const staleWindow = !!view?.window && !checking && !windowOptions.some((option) => option.value === windowKey);

  return <div className="flex flex-col gap-5">
    <div className="flex flex-col items-center gap-2 px-1 pb-2 pt-1 text-center">
      <ComputerIllustration className="size-16" />
      <h2 className="mt-1 text-[16px] font-semibold tracking-tight">Give Mira a computer</h2>
      <p className="max-w-sm text-[12px] leading-relaxed text-muted-foreground">Use this desktop or pair another computer. You stay in control.</p>
    </div>
    {!view ? <div role={error ? 'alert' : 'status'} className="rounded-xl border border-border/60 bg-card/40 p-4 text-[12.5px] text-muted-foreground">
      {error || 'Loading computer settings…'}{error && <Button variant="outline" size="sm" className="mt-3" onClick={() => void refresh()}>Retry</Button>}
    </div> : <>
      {error && <p role="alert" className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-[12px] text-destructive">{error}</p>}
      <TSection icon={<MousePointer2 className="size-3.5" />} title="Computer use">
        <TRow title="Allow computer use" description="Let Mira see the screen and use apps on a computer you choose."
          control={<TSwitch checked={view.enabled} disabled={disabled || (!view.enabled && (!identity || checking || staleWindow || (view.delivery === 'background' && (!view.window || !identity.background_input))))}
            onChange={(enabled) => void change({ enabled })} label="Allow computer use" />} />
      </TSection>
      <TSection icon={<Monitor className="size-3.5" />} title="Computers"
        action={<Button variant="outline" size="sm" disabled={pending} onClick={() => { setNotice(''); setPairing('connect'); }}><Plus />Pair a computer</Button>}>
        <div role="radiogroup" aria-label="Computer to control" className="divide-y divide-border/50">
          <ComputerChoice name="This computer" description="The desktop where Mira is running." selected={view.target === 'local'} disabled={disabled}
            onSelect={() => void change({ target: 'local', window: null, enabled: false })} />
          {view.computers.map((computer) => <ComputerChoice key={computer.id} name={computer.name} description={`${computer.platform} · ${preview ? 'Example computer' : 'Paired'}`}
            selected={view.target === computer.id} disabled={disabled} onSelect={() => void change({ target: computer.id, window: null, enabled: false })}
            action={computer.id === 'configured' ? undefined : <button type="button" disabled={disabled} aria-label={`Unpair ${computer.name}`} title="Unpair computer"
              className="rounded-md p-1.5 text-muted-foreground hover:bg-fg/5 hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50"
              onClick={() => void remove(computer.id)}><Unplug className="size-3.5" /></button>} />)}
        </div>
        <TRow title="Pair from another computer" description="Show a pairing link here, then paste it in Mira on the other computer."
          control={<Button variant="outline" size="sm" disabled={pending} onClick={() => setPairing('share')}>Show pairing link</Button>} />
      </TSection>
      <TNote>Pairing replaces manual tokens and configuration files. You choose which computer Mira can use.</TNote>
      <TSection icon={<MousePointer2 className="size-3.5" />} title="Input & access">
        <TRow title="Input delivery" description={DELIVERY_DESCRIPTION[view.delivery]}
          control={<div role="group" aria-label="Input delivery" className="w-full sm:w-48">
            <Select<Delivery> value={view.delivery} disabled={disabled} onChange={(delivery) => void change({ delivery, enabled: false })} className="h-8 text-[13px]"
              options={[
                { value: 'background', label: 'Background only', hint: 'Keep your pointer and focus.' },
                { value: 'auto', label: 'Automatic', hint: 'Prefer background; allow foreground when needed.' },
                { value: 'foreground', label: 'Foreground', hint: 'Use the visible pointer and keyboard.' },
              ]} />
          </div>} />
        <TRow title="Application window" description={identity?.backend === 'native' ? 'Mira uses the entire desktop in foreground mode.' : staleWindow ? 'This window is no longer available. Choose another before continuing.' : 'Choose the app Mira can see and control. Required for background input.'}
          control={identity?.backend === 'native' ? <span className="text-[12px] text-muted-foreground">Entire desktop</span> : <div role="group" aria-label="Application window" className="w-full sm:w-48">
            <Select value={windowKey} disabled={disabled || checking || !identity} onChange={(value) => { if (!value && view.delivery !== 'background') { void change({ window: null, enabled: false }); return; } const window = windows.find((w) => `${w.id}:${w.epoch}` === value); if (window) void change({ window, enabled: false }); }}
              className="h-8 text-[13px]" placeholder={checking ? 'Loading windows…' : 'Choose a window'}
              options={[...(view.delivery !== 'background' ? [{ value: '', label: 'Entire desktop' }] : []), ...(staleWindow && view.window ? [{ value: windowKey, label: `Unavailable: ${view.window.title || view.window.app}` }] : []), ...windowOptions]} />
          </div>} />
        <TRow title="Connection" description={checking ? 'Checking the selected computer…' : connectionError || (identity ? `${identity.name} · ${identity.background_input ? 'Background input available' : 'Foreground input only'}` : 'Not connected.')}
          control={<Button variant="outline" size="sm" disabled={checking} onClick={() => void check()}>{checking ? <LoaderCircle className="animate-spin" /> : <RefreshCw />}Check connection</Button>} />
        <TRow title="Action approvals" description="Computer actions follow your chat’s permission rules. Pairing doesn’t approve actions."
          control={<span className="inline-flex items-center gap-1.5 text-[12px] text-muted-foreground"><ShieldCheck className="size-3.5" />Permission rules apply</span>} />
      </TSection>
      {view.clients.length > 0 && <TSection icon={<ShieldCheck className="size-3.5" />} title="Computers with access">
        {view.clients.map((computer) => <TRow key={computer.id} title={computer.name || 'Mira computer'} description="Can see and control this computer through its CUA companion."
          control={<Button variant="outline" size="sm" disabled={pending} onClick={() => void remove(computer.id, true)}>Revoke access</Button>} />)}
      </TSection>}
      <p role="status" className="px-1 text-[11.5px] leading-relaxed text-muted-foreground/75">
        {pending ? 'Saving…' : !view.can_change ? 'Pause active chats to change the computer or its access.' : notice || (!view.enabled && identity?.backend === 'native' && view.delivery !== 'foreground' ? 'Select Foreground input to use this computer.' : !view.enabled && view.delivery === 'background' && !view.window ? 'Choose an application window to enable background computer use.' : view.enabled ? `Mira can use ${view.target === 'local' ? 'this computer' : view.computers.find((c) => c.id === view.target)?.name}.` : 'Computer use is off. Pairing is available before you turn it on.')}
      </p>
    </>}
    <PairComputerDialog mode={pairing} preview={preview} onClose={() => setPairing(null)} onPair={async (computer) => {
      await paired(computer); setPairing(null); setNotice(`${computer.name} paired. Choose the computer and an application window, then turn on computer use.`);
    }} />
  </div>;
}

function ComputerChoice({ name, description, selected, disabled, onSelect, action }: {
  name: string; description: string; selected: boolean; disabled: boolean; onSelect: () => void; action?: React.ReactNode;
}) {
  return <div className="flex items-center gap-1 px-3 py-2">
    <button type="button" role="radio" aria-checked={selected} disabled={disabled} onClick={onSelect}
      className="flex min-w-0 flex-1 items-center gap-3 rounded-lg px-1 py-1.5 text-left outline-none hover:bg-fg/[0.025] focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50">
      <span className="flex size-10 shrink-0 items-center justify-center"><ComputerIllustration className="size-9" /></span>
      <span className="min-w-0 flex-1"><span className="block truncate text-[13.5px] font-medium">{name}</span>
        <span className="mt-0.5 block text-[12px] leading-relaxed text-muted-foreground/80">{description}</span></span>
      <span className={cn('mr-1 flex size-4 shrink-0 items-center justify-center rounded-full border', selected ? 'border-mira-blue bg-mira-blue text-mira-on-accent' : 'border-border')}>
        {selected && <Check className="size-2.5" />}
      </span>
    </button>{action}
  </div>;
}
