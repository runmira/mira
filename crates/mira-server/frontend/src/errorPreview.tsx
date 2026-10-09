import { createRoot } from 'react-dom/client';
import { useState } from 'react';
import { LazyMotion, domMax } from 'framer-motion';
import { ErrorNotice } from './components/ErrorNotice';
import { TurnView } from './components/transcript/TurnView';
import { MessageActionsContext } from './components/transcript/EntryView';
import { applyTheme } from './lib/theme';
import type { Entry } from './transcript/entries';
import './styles.css';
applyTheme();
function Preview() {
  const [failed, setFailed] = useState(true);
  const [pending, setPending] = useState(false);
  const [notice, setNotice] = useState('');
  const [limited, setLimited] = useState(false);
  const [offline, setOffline] = useState(false);
  const [actionError, setActionError] = useState(false);
  const [steered, setSteered] = useState(false);
  const [dark, setDark] = useState(document.documentElement.dataset.theme === 'dark');
  const user: Entry = { kind: 'msg', msg: { role: 'user', content: 'Explain the sidebar architecture.' } };
  const body: Entry[] = [
    {kind:'msg',msg:{role:'assistant',content:'The sidebar groups sessions by project and highlights chats that need your attention.'}},
    ...(steered ? [{kind:'msg' as const,msg:{role:'user' as const,content:'Added instructions during the response.',input_intent:'steer' as const}}] : []),
    actionError ? {kind:'error',text:'new chat: network error'} : {kind:'warning',text:limited ? 'provider error: usage limit reached' : 'stream error: connection reset by peer'},
  ];
  return <LazyMotion features={domMax}><main className="min-h-screen bg-background p-6 text-foreground"><div className="mx-auto max-w-2xl space-y-6">
    <div className="flex gap-4 text-sm"><button onClick={() => { const next = !dark; setDark(next); document.documentElement.dataset.theme = next ? 'dark' : 'light'; document.documentElement.classList.toggle('dark', next); }}>Switch to {dark ? 'light' : 'dark'} mode</button><button onClick={() => {setFailed(true); setNotice('');}}>Reset errors</button><button onClick={() => setLimited(v => !v)}>{limited ? 'Show stream failure' : 'Show usage limit'}</button><button onClick={() => setOffline(v => !v)}>{offline ? 'Reconnect' : 'Disconnect'}</button><button onClick={() => {setActionError(v => !v); setNotice('');}}>{actionError ? 'Show response failure' : 'Show action error'}</button><button onClick={() => setSteered(v => !v)}>{steered ? 'Remove steering input' : 'Add steering input'}</button></div>
    <h1 className="text-lg font-semibold">Error recovery preview</h1>
    {failed ? <ErrorNotice title="Couldn't load chats" description="Check that Mira is running, then try again." details="sessions GET 500" pending={pending} onRetry={() => { setPending(true); setTimeout(() => { setPending(false); setFailed(false); }, 500); }} /> : <p role="status" className="text-sm">Chats loaded successfully.</p>}
    <MessageActionsContext.Provider value={{busy:false, edit:(_entry, text)=>setNotice(`Retry targets opening message: ${text}`), restore:()=>{}, fork:null, openImage:()=>{}, retry:()=>setNotice('Retry requested for the original message.')}}>
      <TurnView turn={{user,body}} index={0} timing={{startedAt:Date.now()-15000,endedAt:Date.now()}} usage={null} model="test" expanded={false} isActive={false} onToggle={()=>{}} onDecide={()=>{}} onPlanReply={()=>{}} onAskUserReply={()=>{}} onOpenAgent={()=>{}} onOpenFile={()=>{}} skills={[]} mode="manual" onSetMode={()=>{}} failureRetryEnabled recoveryDisabled={offline} recovery={limited ? {id:'limit-preview',text:'Explain the sidebar architecture.',images:[],engine:'provider',dispatching:false,error:null,recovery:{created_at:Date.now(),reset_at:Date.now()+3600000,scheduled_at:null,snoozed:false}} : undefined} onRecoveryAction={async (_id, action) => setNotice(`Existing usage recovery: ${action}`)} onFailureSettings={()=>setNotice('Connection settings requested.')} />
    </MessageActionsContext.Provider>
    {notice && <p role="status" className="text-sm">{notice}</p>}
  </div></main></LazyMotion>;
}
createRoot(document.getElementById('root')!).render(<Preview />);
