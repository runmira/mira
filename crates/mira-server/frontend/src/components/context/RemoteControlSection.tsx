import { Copy, ExternalLink, Loader2, MoreHorizontal } from 'lucide-react';
import { useState } from 'react';
import { Menu, MenuItem } from '../ui/Menu';
import { useRemoteControl } from '../../hooks/useRemoteControl';
import { remoteAccessAvailable } from '../../lib/remote';

export function RemoteControlSection() {
  const { remote, busy, error, setEnabled } = useRemoteControl();
  const [menuOpen, setMenuOpen] = useState(false);
  const [linkFeedback, setLinkFeedback] = useState<string | null>(null);
  const url = remote?.enabled && remote.hostname ? `https://${remote.hostname}` : null;
  const enabled = remote?.enabled ?? false;
  const disabled = busy || !remote || (!enabled && (!remote.supported || !remoteAccessAvailable()));
  const detail = !remote ? 'Checking remote access…'
    : !remote.supported ? 'Unavailable on this computer'
    : !enabled ? (remoteAccessAvailable() ? 'Access Mira from another device' : 'Sign in to Mira in Settings')
    : remote.status === 'connected' ? 'Connected'
    : remote.status === 'reconnecting' ? 'Reconnecting…' : 'Starting…';
  return <div className="py-2">
    <div className="flex min-h-7 items-center gap-1.5 py-1.5">
      <span className="flex-1 text-[11px] font-bold uppercase tracking-widest text-fg/40">Remote control</span>
      {url && <div className="relative">
        <button type="button" aria-label="Remote control options" aria-haspopup="menu" aria-expanded={menuOpen}
          onClick={() => setMenuOpen(open => !open)}
          className="rounded-md p-1 text-fg/35 transition-colors hover:bg-fg/[0.05] hover:text-fg/70"><MoreHorizontal className="size-3.5" /></button>
        <Menu open={menuOpen} onOpenChange={setMenuOpen} label="Remote control options">
          <MenuItem label="Copy link" icon={<Copy className="size-3.5" />} onSelect={() => {
            void (async () => {
              try { await navigator.clipboard.writeText(url); setLinkFeedback('Link copied.'); }
              catch { setLinkFeedback('Could not copy the link.'); }
            })();
          }} />
          <MenuItem label="Open link" icon={<ExternalLink className="size-3.5" />} onSelect={() => { window.open(url, '_blank', 'noopener,noreferrer'); }} />
        </Menu>
      </div>}
      {busy && <Loader2 aria-hidden="true" className="size-3 animate-spin text-fg/40" />}
      <button type="button" role="switch" aria-label="Remote control" aria-checked={enabled}
        disabled={disabled} onClick={() => { void setEnabled(!enabled).catch(() => {}); }}
        className={`flex h-4 w-7 items-center rounded-full p-0.5 transition-colors disabled:opacity-40 ${enabled ? 'bg-blue-500' : 'bg-fg/15'}`}>
        <span className={`size-3 rounded-full bg-white shadow transition-transform ${enabled ? 'translate-x-3' : ''}`} />
      </button>
    </div>
    <div className="flex items-center gap-1.5 pb-1 text-[12px] leading-5 text-fg/45" role="status">
      {enabled && <span aria-hidden="true" className={`size-1.5 shrink-0 rounded-full ${remote?.status === 'connected' ? 'bg-green-500/75' : 'bg-amber-500/75'}`} />}
      <span>{detail}</span>
    </div>
    {url && <a href={url} target="_blank" rel="noreferrer"
      className="block truncate pb-1 text-[11.5px] text-fg/55 transition-colors hover:text-fg/80 hover:underline">{remote?.hostname}</a>}
    {enabled && linkFeedback && <p role="status" className="mt-1 text-[11px] text-fg/50">{linkFeedback}</p>}
    {(error || remote?.error) && <p role="alert" className="mt-1 text-[11px] text-red-500">{error?.message ?? remote?.error}</p>}
  </div>;
}
