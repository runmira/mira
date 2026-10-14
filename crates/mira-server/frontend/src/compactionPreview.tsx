import {useEffect,useState} from 'react';
import {createRoot} from 'react-dom/client';
import {CompactionCard, type CompactionEntry} from './components/CompactionCard';
import './styles.css';
/** Development fixture for native compaction lifecycle and bounded summaries. */
function Preview() {
 const [dark,setDark]=useState(false);
 useEffect(()=>{document.documentElement.dataset.theme=dark?'dark':'light';},[dark]);
 const entries:CompactionEntry[]=[
  {kind:'compact',state:'running',trigger:'agent',summarized:null,summary:'The agent is retaining the project decisions as it summarizes.'},
  {kind:'compact',state:'done',trigger:'agent',summarized:null,summary:'Project: Mira.\n\nCompleted:\n- Verified ACP compaction lifecycle.\n- Preserved native session ownership.\n\nNext: continue the queued task.\n'+('Remember the current branch and selected model.\n').repeat(35),tokensBefore:28000,tokensAfter:4300},
  {kind:'compact',state:'done',trigger:'agent',summarized:null,summary:null},
  {kind:'compact',state:'failed',trigger:'agent',summarized:null,summary:null,error:'OpenCode could not finish compaction because the provider connection closed.'},
  {kind:'compact',state:'cancelled',trigger:'agent',summarized:null,summary:null},
 ];
 return <div className={dark?'dark':''}><main className="min-h-screen bg-background px-6 py-8 text-foreground"><div className="mx-auto max-w-3xl">
  <header className="mb-8 flex items-center justify-between"><h1 className="text-lg font-medium">Compaction in the transcript</h1><button className="rounded-md border border-border px-3 py-1.5 text-xs" onClick={()=>setDark(value=>!value)}>Switch theme</button></header>
  {entries.map((entry,index)=><section key={index} className="mb-7"><p className="mb-3 text-[13px] leading-relaxed">{['The agent is summarizing context while the session stays active.','The project decisions are preserved for the next turn.','This agent reports completion without exposing summary text.','The compaction attempt did not finish.','The user stopped the compaction.'][index]}</p><CompactionCard entry={entry}/></section>)}
 </div></main></div>;
}
createRoot(document.getElementById('root')!).render(<Preview/>);
