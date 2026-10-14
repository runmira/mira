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
const engines = ['codex', 'claude-code', 'opencode'].map(instance => ({ instance, driver: instance, flavor: 'external', enabled: true, display_name: instance, models: [], state: {state:'ready'} })) as EngineSnapshot[];
function Preview() {
  const [picker, setPicker] = useState(false), [active, setActive] = useState(true);
  const [choice, setChoice] = useState('None');
  const pane = useRef<HTMLDivElement>(null), follow = useRef(false), lastInput = useRef(0);
  const user: Entry = {kind:'msg',msg:{role:'user',content:'Inspect the project'},transcriptTurnIndex:30};
  const body: Entry[] = Array.from({length:12},(_,i)=>({kind:'msg',msg:{role:'assistant',content:`Work detail ${i+1}: inspect and explain the implementation. `.repeat(8)}}));
  body.push({kind:'msg',msg:{role:'assistant',content:active?'Still working.':'Finished the task.'}});
  return <LazyMotion features={domAnimation}><main className="mx-auto max-w-3xl p-5">
    <div className="mb-4 flex items-center gap-3"><button onClick={()=>setPicker(true)}>Open model picker</button><button onClick={()=>setActive(v=>!v)}>Toggle turn completion</button><span>{choice}</span><SessionStatus running={active} unread={false} /><SessionTime running={active} updatedAt={0} /></div>
    <EnginePicker open={picker} onOpenChange={setPicker} engine={{kind:'agent',driver:'opencode',instance:'opencode',display_name:'OpenCode',status:'idle'}} fallbackModel="" engines={engines} agents={[]} onPickProvider={()=>{}} onPickAgent={(driver,model,instance)=>setChoice(`${instance ?? driver}: ${model ?? 'default'}`)} onSetModelOption={()=>{}} />
    <div ref={pane} className="h-[600px] overflow-y-auto border p-4" style={{overflowAnchor:'none'}} onWheel={()=>{follow.current=false;lastInput.current=Date.now();}}>
      <VirtualTranscript pane={pane} identity="reliability-fixture" follow={follow} lastInput={lastInput}>
        {Array.from({length:30},(_,i)=><div key={`history-${i}`} className="py-3"><p>Earlier turn {i+1}</p><p>{'Historical output. '.repeat(25)}</p></div>)}
        <TurnView key="live-turn" turn={{user,body}} index={30} expanded={false} isActive={active} timing={{startedAt:Date.now()-10000,endedAt:active?null:Date.now()}} usage={null} model="test" onToggle={()=>{}} onDecide={()=>{}} onPlanReply={()=>{}} onAskUserReply={()=>{}} onOpenAgent={()=>{}} onOpenFile={()=>{}} skills={[]} mode="manual" onSetMode={()=>{}} />
      </VirtualTranscript>
    </div>
  </main></LazyMotion>;
}
createRoot(document.getElementById('root')!).render(<RuntimeProvider scope={{backend:'fixture',account:'fixture'}}><Preview /></RuntimeProvider>);
