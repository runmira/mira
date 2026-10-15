import { Copy, ExternalLink, Loader2, MoreHorizontal, Radio } from 'lucide-react';
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
    : remote.status === 'connected' ? 'Connected · paired devices can access Mira'
    : remote.status === 'reconnecting' ? 'Reconnecting…' : 'Starting…';
  return <div className="py-2.5">
    <div className="flex items-center gap-2">
      <Radio aria-hidden="true" className="size-3.5 text-fg/40" />
      <span className="flex-1 text-[12px] font-medium text-fg/70">Remote control</span>
      {url && <div className="relative">
        <button type="button" aria-label="Remote control options" aria-haspopup="menu" aria-expanded={menuOpen}
          onClick={() => setMenuOpen(open => !open)}
          className="rounded p-1 text-fg/45 hover:bg-fg/5 hover:text-fg/80"><MoreHorizontal className="size-3.5" /></button>
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
    <p className="mt-1 text-[11px] leading-relaxed text-fg/45" role="status">{detail}</p>
    {enabled && remote?.hostname && <a href={`https://${remote.hostname}`} target="_blank" rel="noreferrer"
      className="mt-1 block truncate text-[11px] text-blue-500 hover:underline">{remote.hostname}</a>}
    {enabled && linkFeedback && <p role="status" className="mt-1 text-[11px] text-fg/50">{linkFeedback}</p>}
    {(error || remote?.error) && <p role="alert" className="mt-1 text-[11px] text-red-500">{error?.message ?? remote?.error}</p>}
  </div>;
}
