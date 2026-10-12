import { useEffect, useRef, useState } from 'react';
import { ArrowLeft, ArrowRight, Check, Copy, LoaderCircle, ShieldCheck } from 'lucide-react';
import { Button } from '../../ui/button';
import { SectionInput } from '../../ui/input';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../../ui/dialog';
import { ComputerIllustration } from './ComputerIllustration';
import { connectComputer, inspectComputerPairing, openComputerPairing, closeComputerPairing,
  type ComputerIdentity, type PairedComputer, type PairingLink } from './api';
import { getRemote, remoteAccessAvailable, turnOnRemote } from '@/lib/remote';

export function PairComputerDialog({ mode, preview, onClose, onPair }: {
  mode: 'connect' | 'share' | null; preview: boolean; onClose: () => void; onPair: (computer: PairedComputer) => Promise<void>;
}) {
  return <Dialog open={mode !== null} onOpenChange={(open) => { if (!open) onClose(); }}>
    {mode && <PairingSteps key={mode} mode={mode} preview={preview} onClose={onClose} onPair={onPair} />}
  </Dialog>;
}

function PairingSteps({ mode, preview, onClose, onPair }: {
  mode: 'connect' | 'share'; preview: boolean; onClose: () => void; onPair: (computer: PairedComputer) => Promise<void>;
}) {
  const [link, setLink] = useState('');
  const [identity, setIdentity] = useState<ComputerIdentity | null>(null);
  const [share, setShare] = useState<PairingLink | null>(null);
  const [error, setError] = useState('');
  const [pending, setPending] = useState(false);
  const [copied, setCopied] = useState(false);
  const [now, setNow] = useState(Date.now());
  const [attempt, setAttempt] = useState(0);
  const active = useRef(true);
  const saving = useRef(false);
  const sharing = mode === 'share';
  const expired = !!share && now / 1000 >= share.expires_at;
  useEffect(() => { active.current = true; return () => { active.current = false; }; }, []);

  useEffect(() => {
    if (!sharing) return;
    let cancelled = false;
    let opened: PairingLink | null = null;
    setPending(true); setError(''); setShare(null);
    async function open() {
      if (preview) return { code: 'AB12CD34EF', expires_at: Date.now() / 1000 + 600, link: 'https://example.runmira.dev/#computer-pair=AB12CD34EF' };
      const remote = await getRemote();
      if (!remote.enabled) {
        if (!remoteAccessAvailable()) throw new Error('Sign in to Mira and enable Remote access in Devices settings to pair another computer.');
        await turnOnRemote(remote);
      }
      // Opening the sheet authorizes remote access, but closing it stops waiting and code creation.
      for (let i = 0; i < 30; i++) {
        if (cancelled) return null;
        const status = await getRemote();
        if (status.status === 'connected' && !status.port_stale) return openComputerPairing();
        if (status.error) throw new Error(status.error);
        await new Promise((resolve) => setTimeout(resolve, 1000));
      }
      throw new Error('Remote access is still connecting. Try showing the link again in a moment.');
    }
    void open().then((result) => {
      opened = result;
      if (cancelled) { if (result && !preview) void closeComputerPairing(result.code).catch(() => {}); return; }
      setShare(result); setPending(false);
    }).catch((e) => { if (!cancelled) { setError((e as Error).message); setPending(false); } });
    return () => { cancelled = true; if (opened && !preview) void closeComputerPairing(opened.code).catch(() => {}); };
  }, [sharing, preview, attempt]);
  useEffect(() => { if (!share) return; const timer = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(timer); }, [share]);

  async function continuePairing(event: React.FormEvent) {
    event.preventDefault(); if (saving.current) return;
    saving.current = true; setPending(true); setError('');
    try {
      const result = preview ? { name: 'Studio Mac', platform: 'macOS', machine_id: 'example', background_input: true, limitation: '', backend: 'cua' as const } : await inspectComputerPairing(link.trim());
      if (active.current) setIdentity(result);
    } catch (e) { if (active.current) setError((e as Error).message); }
    finally { saving.current = false; if (active.current) setPending(false); }
  }
  async function confirm() {
    if (saving.current || !identity) return;
    saving.current = true; setPending(true); setError('');
    try {
      const computer = preview ? { id: 'example', name: identity.name, platform: identity.platform } : await connectComputer(link.trim());
      // Pairing completion must refresh even if the sheet was closed during the request.
      await onPair(computer);
    } catch (e) { if (active.current) setError((e as Error).message); }
    finally { saving.current = false; if (active.current) setPending(false); }
  }

  return <DialogContent className="max-w-[420px] w-[calc(100%-2rem)] gap-5 p-5" showClose={!pending} onEscapeKeyDown={(event) => { if (pending && !sharing) event.preventDefault(); }} onInteractOutside={(event) => { if (pending && !sharing) event.preventDefault(); }}>
    <DialogHeader className="gap-2 pr-6">
      <DialogTitle>{sharing ? 'Pair from another computer' : identity ? 'Confirm computer' : 'Pair a computer'}</DialogTitle>
      <DialogDescription className="text-[12.5px] leading-relaxed">
        {sharing ? 'Open Computer settings in Mira on the other computer and paste this pairing link.' : identity ? 'Check the computer before allowing Mira to use it.' : 'On the other computer, open Mira → Settings → Computer and show a pairing link.'}
      </DialogDescription>
    </DialogHeader>
    {sharing ? <>
      {pending ? <p role="status" className="flex items-center gap-2 py-4 text-[12px] text-muted-foreground"><LoaderCircle className="size-4 animate-spin" />Preparing a secure pairing link…</p> : share && <>
        <div className="rounded-xl border border-border/60 bg-card/40 px-4 py-5 text-center">
          <p className="mb-3 text-[11.5px] text-muted-foreground">{expired ? 'Pairing code expired' : 'Your pairing code'}</p>
          <p className="font-mono text-[24px] font-medium tracking-[0.15em]">{share.code.slice(0, 5)} {share.code.slice(5)}</p>
          <p className="mt-3 text-[11.5px] text-muted-foreground">{expired ? 'Show a new link to continue.' : `Expires in ${Math.max(0, Math.ceil((share.expires_at - now / 1000) / 60))} minutes · Single use`}</p>
        </div>
        <Button variant="outline" size="sm" disabled={expired} onClick={() => { void navigator.clipboard.writeText(share.link).then(() => setCopied(true)).catch(() => setError('Could not copy the link. Select it below and copy it manually.')); }}>
          {copied ? <Check /> : <Copy />}{copied ? 'Copied pairing link' : 'Copy pairing link'}</Button>
        <SectionInput aria-label="Pairing link" readOnly value={share.link} onFocus={(event) => event.target.select()} className="h-8 text-[11px]" />
      </>}
      <p className="flex items-start gap-2 text-[12px] leading-relaxed text-muted-foreground"><ShieldCheck className="mt-0.5 size-3.5 shrink-0" />The link grants computer access to whoever pairs with it. Only share it with a computer you trust. Closing this sheet cancels the link.</p>
      {preview && <PreviewNote />}
      {error && <p role="alert" className="text-[12px] text-destructive">{error}</p>}
      <DialogFooter className="pt-0">{(expired || error) && <Button variant="outline" size="sm" onClick={() => { setCopied(false); setNow(Date.now()); setAttempt((a) => a + 1); }}>Show new link</Button>}<Button variant="outline" size="sm" onClick={onClose}>Done</Button></DialogFooter>
    </> : identity ? <>
      <div className="flex items-center gap-3 rounded-xl border border-border/60 bg-card/40 p-4">
        <ComputerIllustration className="size-11 shrink-0" />
        <div><p className="text-[13.5px] font-medium">{identity.name}</p><p className="mt-0.5 text-[12px] text-muted-foreground">{identity.platform}{preview ? ' · Example computer' : ''}</p></div>
      </div>
      <div className="space-y-2 text-[12px] leading-relaxed text-muted-foreground">
        <p className="flex items-center gap-2"><Check className="size-3.5 text-mira-blue" />{identity.background_input ? 'Background input available' : 'Foreground input only'}</p>
        {identity.limitation && <p>{identity.limitation}</p>}
        <p className="flex items-start gap-2"><ShieldCheck className="mt-0.5 size-3.5 shrink-0" />Pairing lets Mira see this computer’s screen and send input. Computer use stays off until you turn it on.</p>
      </div>
      {preview && <PreviewNote />}{error && <p role="alert" className="text-[12px] text-destructive">{error}</p>}
      <DialogFooter className="justify-between pt-0"><Button disabled={pending} variant="ghost" size="sm" onClick={() => { setIdentity(null); setError(''); }}><ArrowLeft />Back</Button>
        <Button disabled={pending} size="sm" onClick={() => void confirm()}>{pending ? <LoaderCircle className="animate-spin" /> : <ArrowRight />}{pending ? 'Pairing…' : preview ? 'Preview pairing' : 'Pair computer'}</Button></DialogFooter>
    </> : <form onSubmit={(event) => void continuePairing(event)} className="space-y-5">
      <div>
        <label htmlFor="computer-pairing-link" className="mb-2 block text-[12px] font-medium">Pairing link</label>
        <SectionInput id="computer-pairing-link" autoFocus autoComplete="off" spellCheck={false} maxLength={2048} disabled={pending} value={link}
          onChange={(event) => { setLink(event.target.value); setError(''); }} placeholder="Paste pairing link" aria-invalid={!!error} />
        {preview && <p className="mt-2 text-[11.5px] leading-relaxed text-muted-foreground">In this design preview, enter any text to try the flow.</p>}
        {error && <p role="alert" className="mt-2 text-[12px] text-destructive">{error}</p>}
      </div>
      {preview && <PreviewNote />}
      <DialogFooter className="pt-0"><Button disabled={pending} type="button" variant="outline" size="sm" onClick={onClose}>Cancel</Button>
        <Button type="submit" size="sm" disabled={pending || !link.trim()}>{pending ? <LoaderCircle className="animate-spin" /> : <ArrowRight />}{pending ? 'Checking…' : 'Continue'}</Button></DialogFooter>
    </form>}
  </DialogContent>;
}
function PreviewNote() { return <p className="border-t border-border/50 pt-3 text-[11.5px] leading-relaxed text-muted-foreground/75">Design preview. No connection or access is granted.</p>; }
