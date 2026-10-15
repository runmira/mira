import {useEffect,useState} from 'react';
import {createRoot} from 'react-dom/client';
import {CompactionCard, type CompactionEntry} from './components/CompactionCard';
import {UsageRing, type UsageRingData} from './components/UsageRing';
import {applyAcpCompaction} from './lib/acpCompaction';
import type {Entry} from './transcript/entries';
import type {ServerMsg} from './types';
import './styles.css';

type Frame = Extract<ServerMsg, {type: 'acp_compaction'}>;
const frame = (id: string, status: string): Frame => ({type: 'acp_compaction', id, status, summary: null, replace_summary: false, error: null, replace_error: false});

/** The frames the server sends when each agent compacts on its own. */
const REPLAYS: Record<string, {label: string; steps: Frame[]}> = {
  codex: {label: 'Codex auto-compaction', steps: [frame('codex:c1', 'in_progress'), frame('codex:c1', 'completed')]},
  claude: {label: 'Claude Code auto-compaction', steps: [frame('claude:s:compact:1', 'in_progress'), frame('claude:s:compact:1', 'completed')]},
};

/** A Codex chat's ring: 258,400 usable tokens, compacting at 244,800. */
const ring = (used: number): UsageRingData => ({
  used, window: 258_400, compactAt: 244_800, tokens: null, costUsd: null, limitsTitle: null, limits: [],
  onCompact: () => {}, onInspect: () => {},
});

/** Development fixture for native compaction lifecycle and bounded summaries. */
function Preview() {
 const [dark,setDark]=useState(true);
 useEffect(()=>{document.documentElement.dataset.theme=dark?'dark':'light';},[dark]);
 const [timeline,setTimeline]=useState<Entry[]>([]);
 const [note,setNote]=useState('Nothing replayed yet.');
 async function replay(key: string) {
  const {label, steps} = REPLAYS[key];
  setTimeline([]);
  for (const [i, f] of steps.entries()) {
   setNote(`${label}: ${f.status === 'in_progress' ? 'started' : 'finished'} (frame ${i + 1} of ${steps.length})`);
   setTimeline(prev => applyAcpCompaction(prev, f));
   await new Promise(r => setTimeout(r, 1600));
  }
 }
 const entries:CompactionEntry[]=[
  {kind:'compact',state:'running',trigger:'agent',summarized:null,summary:'The agent is retaining the project decisions as it summarizes.'},
  {kind:'compact',state:'done',trigger:'agent',summarized:null,summary:'Project: Mira.\n\nCompleted:\n- Verified ACP compaction lifecycle.\n- Preserved native session ownership.\n\nNext: continue the queued task.\n'+('Remember the current branch and selected model.\n').repeat(35),tokensBefore:28000,tokensAfter:4300},
  {kind:'compact',state:'done',trigger:'agent',summarized:null,summary:null},
  {kind:'compact',state:'failed',trigger:'agent',summarized:null,summary:null,error:'OpenCode could not finish compaction because the provider connection closed.'},
  {kind:'compact',state:'cancelled',trigger:'agent',summarized:null,summary:null},
 ];
 return <div className={dark?'dark':''}><main className="min-h-screen bg-background px-6 py-8 text-foreground"><div className="mx-auto max-w-3xl">
  <header className="mb-8 flex items-center justify-between"><h1 className="text-lg font-medium">Compaction in the transcript</h1><button className="rounded-md border border-border px-3 py-1.5 text-xs" onClick={()=>setDark(value=>!value)}>Switch theme</button></header>

  <section className="mb-10 rounded-xl border border-border/60 p-4">
   <h2 className="mb-1 text-[13px] font-medium">Agent compactions (new)</h2>
   <p className="mb-3 text-[12px] text-muted-foreground">Replays the frames the server now sends when an agent compacts by itself. Before this change these were dropped and nothing appeared.</p>
   <div className="mb-4 flex gap-2">
    {Object.entries(REPLAYS).map(([key, r]) => <button key={key} className="rounded-md border border-border px-3 py-1.5 text-xs hover:bg-muted/50" onClick={() => void replay(key)}>Replay {r.label}</button>)}
   </div>
   <p className="mb-3 text-[11.5px] text-muted-foreground" role="status">{note}</p>
   {timeline.map((entry, i) => entry.kind === 'compact' ? <CompactionCard key={i} entry={entry as CompactionEntry}/> : null)}
  </section>

  <section className="mb-10 rounded-xl border border-border/60 p-4">
   <h2 className="mb-1 text-[13px] font-medium">Context ring in a Codex chat (new)</h2>
   <p className="mb-4 text-[12px] text-muted-foreground">Hover a ring. Agent chats now show where the agent compacts by itself. Left: your busiest Codex chat's peak. Right: past the threshold.</p>
   <div className="flex items-center gap-10">
    <div className="flex items-center gap-2 text-[12px]"><UsageRing data={ring(233_434)}/> 233k used</div>
    <div className="flex items-center gap-2 text-[12px]"><UsageRing data={ring(246_000)}/> 246k used</div>
   </div>
  </section>

  <h2 className="mb-4 text-[13px] font-medium">Every divider state</h2>
  {entries.map((entry,index)=><section key={index} className="mb-7"><p className="mb-3 text-[13px] leading-relaxed">{['The agent is summarizing context while the session stays active.','The project decisions are preserved for the next turn.','This agent reports completion without exposing summary text.','The compaction attempt did not finish.','The user stopped the compaction.'][index]}</p><CompactionCard entry={entry}/></section>)}
 </div></main></div>;
}
createRoot(document.getElementById('root')!).render(<Preview/>);
