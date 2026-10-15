import { useState } from 'react';
import { Menu, MenuItem } from '../ui/Menu';
import { Tip } from '../ui/Tip';
import { Copy, ExternalLink, Loader2, MoreHorizontal } from 'lucide-react';
import { useRemoteControl } from '../../hooks/useRemoteControl';
import { remoteAccessAvailable } from '../../lib/remote';

export function RemoteControlSection() {
  const { remote, busy, error, setEnabled } = useRemoteControl();
  const [menuOpen, setMenuOpen] = useState(false);
  const [feedback, setFeedback] = useState<string | null>(null);
  const url = remote?.enabled && remote.hostname ? `https://${remote.hostname}` : null;
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
      {url && <div className="relative">
        <Tip label="Remote control options" align="end"><button type="button" aria-label="Remote control options" aria-haspopup="menu" aria-expanded={menuOpen} onClick={() => setMenuOpen(value => !value)} className="rounded-full p-1 text-fg/40 hover:bg-fg/5 hover:text-fg/70"><MoreHorizontal className="size-3.5" /></button></Tip>
        <Menu open={menuOpen} onOpenChange={setMenuOpen} label="Remote control options">
          <MenuItem label="Copy link" icon={<Copy className="size-3.5" />} onSelect={() => { void (async () => { try { await navigator.clipboard.writeText(url); setFeedback('Link copied.'); } catch { setFeedback('Could not copy the link.'); } })(); }} />
          <MenuItem label="Open link" icon={<ExternalLink className="size-3.5" />} onSelect={() => { window.open(url, '_blank', 'noopener,noreferrer'); }} />
        </Menu>
      </div>}
      {busy && <Loader2 aria-hidden="true" className="size-3 animate-spin text-fg/40" />}
      <button type="button" role="switch" aria-label="Remote control" aria-checked={enabled}
        disabled={disabled} onClick={() => { void setEnabled(!enabled).catch(() => {}); }}
        className={`flex h-4 w-7 shrink-0 items-center rounded-full p-0.5 transition-colors disabled:opacity-40 ${enabled ? 'bg-blue-500' : 'bg-fg/15'}`}>
        <span className={`size-3 rounded-full bg-white shadow transition-transform ${enabled ? 'translate-x-3' : ''}`} />
      </button>
    </div>
    {detail && <p role="status" className="pb-1 text-[12px] leading-5 text-fg/45">{detail}</p>}
    {enabled && feedback && <p role="status" className="mt-1 text-[11px] text-fg/50">{feedback}</p>}
    {(error || remote?.error) && <p role="alert" className="mt-1 text-[11px] text-red-500">{error?.message ?? remote?.error}</p>}
  </div>;
}
