/** Development-only sign-in states. No Codex process or OAuth request is started. */
import {useEffect,useState} from 'react';
import {createRoot} from 'react-dom/client';
import {CodexAccountPanel} from './components/settings/CodexAccountPanel';
import {AgentIcon} from './components/AgentIcon';
import type {CodexAccountView} from './api';
import './styles.css';
const base:CodexAccountView={connection_id:'preview-connection',phase:'signed_out',account:null,requires_openai_auth:true,requirements:null,requirements_supported:true,login_id:null,auth_url:null,user_code:null,error:null,revision:1};
let account={...base};
const originalFetch=window.fetch.bind(window);
window.fetch=async(input,init)=>{
  const path=typeof input==='string'?input:input instanceof URL?input.pathname:input.url;
  if(!path.startsWith('/api/engines/codex-preview/account'))return originalFetch(input,init);
  if(init?.method==='POST')account={...base,phase:'pending',login_id:'preview-login',auth_url:'https://auth.openai.com/codex/device',user_code:'ABCD-1234',revision:2};
  if(init?.method==='DELETE')account={...base,phase:'cancelled',revision:3};
  return new Response(JSON.stringify(account),{headers:{'content-type':'application/json'}});
};
function Preview(){
  const [key,setKey]=useState(0),[dark,setDark]=useState(false);
  useEffect(()=>{document.documentElement.dataset.theme=dark?'dark':'light';},[dark]);
  function show(phase:CodexAccountView['phase']){
    account={...base,phase};
    if(phase==='pending')account={...account,login_id:'preview-login',auth_url:'https://auth.openai.com/codex/device',user_code:'ABCD-1234'};
    if(phase==='authenticated')account={...account,account:{kind:'chatgpt',email:'demo@example.test',plan:'Plus'}};
    if(phase==='failed')account={...account,error:'Sign-in was denied. Try again.'};
    setKey(k=>k+1);
  }
  return <main className="mx-auto max-w-xl p-6"><div className="mb-5 flex flex-wrap gap-2 text-xs">{(['signed_out','pending','authenticated','failed','cancelled','unsupported'] as const).map(phase=><button key={phase} className="rounded border border-border px-2 py-1" onClick={()=>show(phase)}>{phase.replace('_',' ')}</button>)}<button className="rounded border border-border px-2 py-1" onClick={()=>setDark(v=>!v)}>{dark?'Light':'Dark'}</button></div><section className="rounded-xl border border-border bg-background p-4"><header className="mb-4 flex items-center gap-2"><AgentIcon kind="codex" name="Codex"/><div><h1 className="font-medium">Codex</h1><p className="text-xs text-muted-foreground">External agent settings · Preview only</p></div></header><CodexAccountPanel key={key} instance="codex-preview" enabled onRefresh={()=>{}}/></section></main>;
}
createRoot(document.getElementById('root')!).render(<Preview/>);
