import { acpOptionsToDescriptors } from './lib/acpOptions';
import type { AcpConfigOption } from './types';
import { SessionStatus, SessionTime } from './components/sidebar/sessionRows';
/** Development-only fixture for provider rail and transcript recovery. */
import { useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { LazyMotion, domAnimation } from 'framer-motion';
import { RuntimeProvider } from './state/RuntimeProvider';
import { EnginePicker } from './components/EnginePicker';
import { VirtualTranscript } from './components/VirtualTranscript';
import { TurnView } from './components/transcript/TurnView';
import type { Entry } from './transcript/entries';
import type { EngineSnapshot } from './api';
import './styles.css';
const engines = ['codex', 'claude-code', 'opencode'].map(instance => ({ instance, driver: instance, flavor: 'external', enabled: true, display_name: ({codex:'Codex','claude-code':'Claude Code',opencode:'OpenCode'} as Record<string,string>)[instance], models: [], state: {state:'ready'} })) as EngineSnapshot[];
function Preview() {
  const [picker, setPicker] = useState(false), [active, setActive] = useState(true);
  const [selectedAgent, setSelectedAgent] = useState('opencode');
  const [selectedModel, setSelectedModel] = useState<string | null>(null);
  const [codexOptions, setCodexOptions] = useState<Record<string,string>>({reasoning_effort:'low',service_tier:''});
  const codexConfig: AcpConfigOption[] = [
    {id:'model',name:'Model',category:'model',current:selectedModel,values:[{value:'gpt-6.1-sol',name:'GPT-6.1-Sol'},{value:'gpt-6-astra',name:'GPT-6-Astra'}]},
    {id:'reasoning_effort',name:'Reasoning',category:'reasoning_effort',current:codexOptions.reasoning_effort,values:['low','medium','high','xhigh','max','ultra'].map(value=>({value,name:value}))},
    {id:'service_tier',name:'Speed',category:'service_tier',current:codexOptions.service_tier,values:[{value:'',name:'Default'},{value:'priority',name:'Fast'}]},
  ];
  const chosenAgent = engines.find(row => row.driver === selectedAgent)!;
  const pane = useRef<HTMLDivElement>(null), follow = useRef(false), lastInput = useRef(0);
  const user: Entry = {kind:'msg',msg:{role:'user',content:'Inspect the project'},transcriptTurnIndex:30};
  const body: Entry[] = Array.from({length:12},(_,i)=>({kind:'msg',msg:{role:'assistant',content:`Work detail ${i+1}: inspect and explain the implementation. `.repeat(8)}}));
  body.push({kind:'msg',msg:{role:'assistant',content:active?'Still working.':'Finished the task.'}});
  return <LazyMotion features={domAnimation}><main className="mx-auto max-w-3xl p-5">
    <div className="mb-4 flex items-center gap-3"><button onClick={()=>setPicker(true)}>Open model picker</button><button onClick={()=>{setSelectedAgent('codex');setSelectedModel('gpt-6.1-sol');setPicker(true);}}>Codex options</button><button onClick={()=>setActive(v=>!v)}>Toggle turn completion</button><SessionStatus running={active} unread={false} /><SessionTime running={active} updatedAt={0} /></div>
    <EnginePicker open={picker} onOpenChange={setPicker} engine={{kind:'agent',driver:selectedAgent,instance:selectedAgent,display_name:chosenAgent.display_name,model:selectedModel,status:'idle'}} fallbackModel="" engines={engines} agents={[]} agentConfig={selectedAgent==='codex'?codexConfig:null} agentDescriptors={selectedAgent==='codex'?(acpOptionsToDescriptors(codexConfig)?.filter(o=>o.id!=='model')??[]):null} onPickProvider={()=>{}} onPickAgent={(driver,model)=>{setSelectedAgent(driver);setSelectedModel(model);}} onSetModelOption={(id,value)=>setCodexOptions(previous=>({...previous,[id]:value}))} />
    <div ref={pane} className="h-[600px] overflow-y-auto border p-4" style={{overflowAnchor:'none'}} onWheel={()=>{follow.current=false;lastInput.current=Date.now();}}>
      <VirtualTranscript pane={pane} identity="reliability-fixture" follow={follow} lastInput={lastInput}>
        {Array.from({length:30},(_,i)=><div key={`history-${i}`} className="py-3"><p>Earlier turn {i+1}</p><p>{'Historical output. '.repeat(25)}</p></div>)}
        <TurnView key="live-turn" turn={{user,body}} index={30} expanded={false} isActive={active} timing={{startedAt:Date.now()-10000,endedAt:active?null:Date.now()}} usage={null} model="test" onToggle={()=>{}} onDecide={()=>{}} onPlanReply={()=>{}} onAskUserReply={()=>{}} onOpenAgent={()=>{}} onOpenFile={()=>{}} skills={[]} mode="manual" onSetMode={()=>{}} />
      </VirtualTranscript>
    </div>
  </main></LazyMotion>;
}
createRoot(document.getElementById('root')!).render(<RuntimeProvider scope={{backend:'fixture',account:'fixture'}}><Preview /></RuntimeProvider>);
