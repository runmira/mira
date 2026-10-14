import { useEffect, useRef, useState } from 'react';
import { Check, Copy, ExternalLink, Loader2, RefreshCw } from 'lucide-react';
import { cancelCodexLogin, getCodexAccount, startCodexLogin, type CodexAccountView } from '../../api';
import { openExternal } from '../../lib/desktop';
import { PrivateText } from '../PrivateText';

export function CodexAccountPanel({instance,enabled,onRefresh}: {instance:string;enabled:boolean;onRefresh:()=>void}) {
  const [view,setView] = useState<CodexAccountView|null>(null);
  const [error,setError] = useState<string|null>(null);
  const [busy,setBusy] = useState(false), [copied,setCopied] = useState(false);
  const epoch = useRef(0), operating = useRef(false), mounted = useRef(false);
  const notify = useRef(onRefresh); notify.current=onRefresh;
  const lastIdentity = useRef<string|null>(null);
  const pending = view?.phase==='pending';
  useEffect(()=>{
    mounted.current=true;operating.current=false;setBusy(false);setView(null);setError(null);setCopied(false);lastIdentity.current=null;
    const owner=++epoch.current;
    let timer: ReturnType<typeof setTimeout>;
    let cancelled=false;
    async function poll() {
      const requestEpoch=epoch.current;
      if (!operating.current && enabled) {
        try {
          const next=await getCodexAccount(instance);
          if (!cancelled && requestEpoch===epoch.current) {setView(next);setError(null);}
        } catch(e) {if (!cancelled && requestEpoch===epoch.current) setError((e as Error).message);}
      }
      if (!cancelled) timer=setTimeout(poll,2000);
    }
    if (enabled) void poll();
    return ()=>{cancelled=true;mounted.current=false;clearTimeout(timer);if(epoch.current>=owner)epoch.current++;};
  },[instance,enabled]);
  useEffect(()=>{
    if (!view || view.phase==='pending') return;
    const identity=JSON.stringify([view.account,view.requires_openai_auth,view.requirements]);
    if (lastIdentity.current!==identity) {lastIdentity.current=identity;notify.current();}
  },[view]);

  async function action(kind:'browser'|'device'|'cancel'|'refresh') {
    const requestEpoch=++epoch.current;
    operating.current=true;setBusy(true);setError(null);setCopied(false);
    try {
      const next=kind==='cancel'?await cancelCodexLogin(instance,view!.connection_id,view!.login_id!):kind==='refresh'?await getCodexAccount(instance,true):await startCodexLogin(instance,kind);
      if (!mounted.current || requestEpoch!==epoch.current) return;
      setView(next);
      if(kind==='browser' && next.phase==='pending' && next.auth_url) await openExternal(next.auth_url);
    } catch(e) {if(mounted.current && requestEpoch===epoch.current)setError((e as Error).message);}
    finally {if(mounted.current && requestEpoch===epoch.current){operating.current=false;setBusy(false);}}
  }
  const button='inline-flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1.5 text-xs hover:bg-muted disabled:opacity-50';
  return <div className="rounded-lg border border-border/60 p-3 text-xs" aria-label="Codex sign-in">
    <div className="flex items-center justify-between gap-2">
      <span className="font-medium">Sign-in</span>
      <button type="button" className={button} disabled={busy||pending||!enabled} onClick={()=>void action('refresh')} aria-label="Check Codex account"><RefreshCw className="size-3.5"/>Check</button>
    </div>
    <div className="mt-2" role="status">
      {(busy&&!pending||!view&&!error&&enabled)?<span className="inline-flex items-center gap-1.5"><Loader2 className="size-3.5 animate-spin"/>Checking Codex…</span>:
        pending?<span className="inline-flex items-center gap-1.5"><Loader2 className="size-3.5 animate-spin"/>Waiting for sign-in…</span>:
        view?.phase==='authenticated'?<span className="inline-flex items-center gap-1.5"><Check className="size-3.5"/><PrivateText text={view.account?.email?`${view.account.email}${view.account.plan?` · ${view.account.plan}`:''}`:view.account?.kind==='apiKey'?'API key configured':view.requires_openai_auth===false?'Authentication not required':'Signed in'}/></span>:
        view?.phase==='cancelled'?'Sign-in cancelled.':view?.phase==='unsupported'?'This Codex version does not support this sign-in flow. Try another flow, or sign in using this instance’s CLI and press Check.':view?.phase==='failed'?'Sign-in could not finish.':enabled?'Not signed in.':'Enable this instance to sign in.'}
    </div>
    {(error||view?.error)&&<p role="alert" className="mt-2 text-destructive"><PrivateText text={error??view?.error??''}/></p>}
    {pending&&view?.user_code&&<div className="mt-2 flex items-center gap-2"><code className="select-text rounded bg-muted px-2 py-1 font-mono">{view.user_code}</code><button type="button" className={button} onClick={()=>void navigator.clipboard.writeText(view.user_code!).then(()=>setCopied(true)).catch(()=>setError('Copy failed. Select the code and copy it manually.'))}><Copy className="size-3"/>{copied?'Copied':'Copy code'}</button></div>}
    <div className="mt-3 flex flex-wrap items-center gap-2">
      {pending?<><button type="button" className={button} disabled={busy||!view?.auth_url} onClick={()=>view?.auth_url&&void openExternal(view.auth_url).catch(e=>setError((e as Error).message))}><ExternalLink className="size-3.5"/>Open sign-in</button><button type="button" className={button} disabled={busy} onClick={()=>void action('cancel')}>Cancel</button></>:
        <><button type="button" className={button} disabled={busy||!enabled} onClick={()=>void action('browser')}>{view?.phase==='authenticated'?'Switch account':'Sign in with ChatGPT'}</button><button type="button" className={button} disabled={busy||!enabled} onClick={()=>void action('device')}>Use device code</button></>}
    </div>
    <p className="mt-2 text-muted-foreground">Uses this Codex instance’s CLI login. Credentials stay with Codex.</p>
  </div>;
}
