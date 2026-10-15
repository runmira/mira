import { Loader2 } from 'lucide-react';
import { useRemoteControl } from '../../hooks/useRemoteControl';
import { remoteAccessAvailable } from '../../lib/remote';

export function RemoteControlSection() {
  const { remote, busy, error, setEnabled } = useRemoteControl();
  const enabled = remote?.enabled ?? false;
  const disabled = busy || !remote || (!enabled && (!remote.supported || !remoteAccessAvailable()));
  const detail = !remote ? 'Checking remote access…'
    : !remote.supported ? 'Unavailable on this computer'
    : !enabled && !remoteAccessAvailable() ? 'Sign in to Mira in Settings'
    : enabled && remote.status === 'reconnecting' ? 'Reconnecting…'
    : enabled && remote.status === 'starting' ? 'Starting…' : null;
  return <div className="py-2">
    <div className="flex min-h-7 items-center gap-2 py-1.5">
      <span className="flex-1 text-[11px] font-bold uppercase tracking-widest text-fg/40">Remote control</span>
      {busy && <Loader2 aria-hidden="true" className="size-3 animate-spin text-fg/40" />}
      <button type="button" role="switch" aria-label="Remote control" aria-checked={enabled}
        disabled={disabled} onClick={() => { void setEnabled(!enabled).catch(() => {}); }}
        className={`flex h-4 w-7 shrink-0 items-center rounded-full p-0.5 transition-colors disabled:opacity-40 ${enabled ? 'bg-blue-500' : 'bg-fg/15'}`}>
        <span className={`size-3 rounded-full bg-white shadow transition-transform ${enabled ? 'translate-x-3' : ''}`} />
      </button>
    </div>
    {detail && <p role="status" className="pb-1 text-[12px] leading-5 text-fg/45">{detail}</p>}
    {(error || remote?.error) && <p role="alert" className="mt-1 text-[11px] text-red-500">{error?.message ?? remote?.error}</p>}
  </div>;
}
