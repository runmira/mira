import { createRoot } from 'react-dom/client';
import { ToolGroup } from './components/ToolGroup';
import { Markdown } from './components/Markdown';
import './styles.css';
const entries = ['screenshot','left_click','key'].map((action,i)=>({call:{id:`preview-${i}`,type:'function' as const,function:{name:'mcp__mira__computer',arguments:JSON.stringify({action,...(i===1?{coordinate:[809,99]}:{})})}},preview:null,status:'complete' as const,result:{call_id:`preview-${i}`,content:i===2?'':JSON.stringify({content:[{type:'text',text:i===0?'Captured the selected window.':'Clicked at (809, 99).'}]})}}));
createRoot(document.getElementById('root')!).render(<main className="mx-auto max-w-3xl px-8 py-16 space-y-6"><p className="text-xs text-muted-foreground">Worked for 39s · 3 calls</p><div className="transcript-work"><div data-assistant-message><Markdown text="I’ll check the selected window and open Spotify." /></div></div><ToolGroup entries={entries}/><div data-assistant-message><Markdown text="Computer control is working. Spotify is open." /></div></main>);
